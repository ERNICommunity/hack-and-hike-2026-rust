//! CPU1 BMI270 acquisition and 6-axis orientation fusion.
//!
//! The BMI270 shares the runtime system-I2C bus with touch. The raw I2C
//! peripheral remains owned by `system_i2c`; this service receives only the
//! CPU1-local async bus handle. CPU0 never receives raw accelerometer/gyroscope
//! samples. Instead, [`LATEST`] is a latest-value snapshot: each fusion update
//! replaces the previous one and CPU0 samples it at its own presentation rate.

use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use embassy_time::{Duration, Timer};

use crate::system_i2c::SystemI2cBus;

/// Default accelerometer/gyroscope acquisition target.
pub const DEFAULT_SENSOR_HZ: u32 = 100;
/// Fusion currently runs once per acquired accel/gyro sample.
pub const DEFAULT_FUSION_HZ: u32 = 100;

/// Runtime-tunable acquisition/fusion parameters.
///
/// `fusion_dt_seconds` should track `sample_period` while fusion is performed
/// once per sensor sample. Keeping the two fields explicit makes later rate
/// decoupling possible without changing the task API.
#[derive(Clone, Copy)]
pub struct Config {
    pub sample_period: Duration,
    pub fusion_dt_seconds: f32,
    pub fusion_alpha: f32,
}

pub const DEFAULT_CONFIG: Config = Config {
    sample_period: Duration::from_millis(10),
    fusion_dt_seconds: 0.010,
    fusion_alpha: 0.98,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum Status {
    Starting = 0,
    Running = 1,
    Degraded = 2,
    Fault = 3,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Orientation {
    pub roll_deg: f32,
    pub pitch_deg: f32,
    /// Gyro-integrated heading. Without magnetometer fusion this will drift.
    pub yaw_deg: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct Snapshot {
    pub revision: u32,
    pub status: Status,
    pub orientation: Orientation,
    pub read_errors: u32,
}

static LATEST: Signal<CriticalSectionRawMutex, Snapshot> = Signal::new();

/// Take the newest orientation/status snapshot, if CPU1 published one since the
/// previous take. Multiple CPU1 updates collapse to one latest value.
pub fn take_latest() -> Option<Snapshot> {
    LATEST.try_take()
}

const BMI270_ADDR: u8 = 0x69;
const CHIP_ID: u8 = 0x24;

const REG_CHIP_ID: u8 = 0x00;
const REG_ACC_X_LSB: u8 = 0x0C;
const REG_INTERNAL_STATUS: u8 = 0x21;
const REG_ACC_CONF: u8 = 0x40;
const REG_ACC_RANGE: u8 = 0x41;
const REG_GYR_CONF: u8 = 0x42;
const REG_GYR_RANGE: u8 = 0x43;
const REG_INIT_CTRL: u8 = 0x59;
const REG_INIT_ADDR_0: u8 = 0x5B;
const REG_INIT_DATA: u8 = 0x5E;
const REG_PWR_CONF: u8 = 0x7C;
const REG_PWR_CTRL: u8 = 0x7D;
const REG_CMD: u8 = 0x7E;

const CMD_SOFT_RESET: u8 = 0xB6;
const CONFIG_LOAD_OK: u8 = 0x01;

// 100 Hz, performance filter, normal bandwidth/averaging.
const ACC_CONF_100HZ: u8 = 0xA8;
const GYR_CONF_100HZ: u8 = 0xA8;
const ACC_RANGE_4G: u8 = 0x01;
const GYR_RANGE_500DPS: u8 = 0x02;
const PWR_CTRL_ACC_GYR: u8 = 0x06;

const ACC_G_PER_LSB: f32 = 4.0 / 32768.0;
const GYR_DPS_PER_LSB: f32 = 500.0 / 32768.0;
const INIT_RETRY: Duration = Duration::from_secs(1);
const SENSOR_STARTUP: Duration = Duration::from_millis(50);
const MAX_CONSECUTIVE_READ_ERRORS: u8 = 10;

#[derive(Clone, Copy, Debug)]
enum Error {
    Bus,
    ChipId(u8),
    ConfigStatus(u8),
}

#[derive(Clone, Copy)]
struct RawSample {
    accel_g: [f32; 3],
    gyro_dps: [f32; 3],
}

struct Bmi270 {
    bus: SystemI2cBus,
}

impl Bmi270 {
    const fn new(bus: SystemI2cBus) -> Self {
        Self { bus }
    }

    async fn write_register(&self, register: u8, value: u8) -> Result<(), Error> {
        let mut i2c = self.bus.lock().await;
        i2c.write_async(BMI270_ADDR, &[register, value])
            .await
            .map_err(|_| Error::Bus)
    }

    async fn read_register(&self, register: u8) -> Result<u8, Error> {
        let mut value = [0u8; 1];
        let mut i2c = self.bus.lock().await;
        i2c.write_read_async(BMI270_ADDR, &[register], &mut value)
            .await
            .map_err(|_| Error::Bus)?;
        Ok(value[0])
    }

    async fn upload_config(&self) -> Result<(), Error> {
        for (offset, chunk) in BMI270_MAXIMUM_FIFO_CONFIG.chunks(32).enumerate() {
            let byte_offset = offset * 32;
            let word_address = byte_offset >> 1;
            let address = [
                REG_INIT_ADDR_0,
                (word_address & 0x0F) as u8,
                (word_address >> 4) as u8,
            ];

            let mut packet = [0u8; 33];
            packet[0] = REG_INIT_DATA;
            packet[1..1 + chunk.len()].copy_from_slice(chunk);

            // Keep the init-address and matching data transaction together so
            // another CPU1 system-I2C client cannot change the sensor state
            // between the two writes.
            let mut i2c = self.bus.lock().await;
            i2c.write_async(BMI270_ADDR, &address)
                .await
                .map_err(|_| Error::Bus)?;
            i2c.write_async(BMI270_ADDR, &packet[..1 + chunk.len()])
                .await
                .map_err(|_| Error::Bus)?;
        }

        Ok(())
    }

    async fn initialize(&self) -> Result<(), Error> {
        let chip_id = self.read_register(REG_CHIP_ID).await?;
        if chip_id != CHIP_ID {
            return Err(Error::ChipId(chip_id));
        }

        self.write_register(REG_CMD, CMD_SOFT_RESET).await?;
        Timer::after(Duration::from_millis(2)).await;

        // BMI270 feature configuration must be loaded with advanced power save
        // disabled. One millisecond comfortably exceeds the 450 us minimum.
        self.write_register(REG_PWR_CONF, 0x00).await?;
        Timer::after(Duration::from_millis(1)).await;
        self.write_register(REG_INIT_CTRL, 0x00).await?;
        self.upload_config().await?;
        Timer::after(Duration::from_millis(1)).await;
        self.write_register(REG_INIT_CTRL, 0x01).await?;

        let mut last_status = 0u8;
        let mut initialized = false;
        for _ in 0..20 {
            Timer::after(Duration::from_millis(1)).await;
            last_status = self.read_register(REG_INTERNAL_STATUS).await?;
            if last_status & 0x0F == CONFIG_LOAD_OK {
                initialized = true;
                break;
            }
        }
        if !initialized {
            return Err(Error::ConfigStatus(last_status));
        }

        self.write_register(REG_ACC_CONF, ACC_CONF_100HZ).await?;
        self.write_register(REG_ACC_RANGE, ACC_RANGE_4G).await?;
        self.write_register(REG_GYR_CONF, GYR_CONF_100HZ).await?;
        self.write_register(REG_GYR_RANGE, GYR_RANGE_500DPS).await?;
        self.write_register(REG_PWR_CTRL, PWR_CTRL_ACC_GYR).await?;
        Timer::after(SENSOR_STARTUP).await;

        Ok(())
    }

    async fn read_sample(&self) -> Result<RawSample, Error> {
        let mut bytes = [0u8; 12];
        let mut i2c = self.bus.lock().await;
        i2c.write_read_async(BMI270_ADDR, &[REG_ACC_X_LSB], &mut bytes)
            .await
            .map_err(|_| Error::Bus)?;
        drop(i2c);

        let acc = [
            i16::from_le_bytes([bytes[0], bytes[1]]),
            i16::from_le_bytes([bytes[2], bytes[3]]),
            i16::from_le_bytes([bytes[4], bytes[5]]),
        ];
        let gyr = [
            i16::from_le_bytes([bytes[6], bytes[7]]),
            i16::from_le_bytes([bytes[8], bytes[9]]),
            i16::from_le_bytes([bytes[10], bytes[11]]),
        ];

        Ok(RawSample {
            accel_g: [
                f32::from(acc[0]) * ACC_G_PER_LSB,
                f32::from(acc[1]) * ACC_G_PER_LSB,
                f32::from(acc[2]) * ACC_G_PER_LSB,
            ],
            gyro_dps: [
                f32::from(gyr[0]) * GYR_DPS_PER_LSB,
                f32::from(gyr[1]) * GYR_DPS_PER_LSB,
                f32::from(gyr[2]) * GYR_DPS_PER_LSB,
            ],
        })
    }
}

#[derive(Clone, Copy)]
struct Fusion {
    orientation: Orientation,
    initialized: bool,
}

impl Fusion {
    const fn new() -> Self {
        Self {
            orientation: Orientation {
                roll_deg: 0.0,
                pitch_deg: 0.0,
                yaw_deg: 0.0,
            },
            initialized: false,
        }
    }

    fn update(&mut self, sample: RawSample, dt_seconds: f32, alpha: f32) -> Orientation {
        let [ax, ay, az] = sample.accel_g;
        let [gx, gy, gz] = sample.gyro_dps;

        let acc_roll = radians_to_degrees(atan2_approx(ay, az));
        let acc_pitch = radians_to_degrees(atan2_approx(-ax, sqrt_approx(ay * ay + az * az)));

        if !self.initialized {
            self.orientation.roll_deg = acc_roll;
            self.orientation.pitch_deg = acc_pitch;
            self.orientation.yaw_deg = 0.0;
            self.initialized = true;
            return self.orientation;
        }

        let alpha = clamp_f32(alpha, 0.0, 1.0);
        let accel_weight = 1.0 - alpha;
        self.orientation.roll_deg = alpha * (self.orientation.roll_deg + gx * dt_seconds)
            + accel_weight * acc_roll;
        self.orientation.pitch_deg = alpha * (self.orientation.pitch_deg + gy * dt_seconds)
            + accel_weight * acc_pitch;
        self.orientation.yaw_deg = wrap_degrees(self.orientation.yaw_deg + gz * dt_seconds);
        self.orientation
    }
}

fn publish(
    revision: &mut u32,
    status: Status,
    orientation: Orientation,
    read_errors: u32,
) {
    *revision = revision.wrapping_add(1);
    LATEST.signal(Snapshot {
        revision: *revision,
        status,
        orientation,
        read_errors,
    });
}

/// CPU1 acquisition/fusion task.
///
/// At the default configuration the BMI270 is sampled at ~100 Hz and fusion is
/// updated at the same rate. CPU0 consumes only [`Snapshot`] values, normally at
/// 25 Hz, so no raw sample stream crosses cores.
#[embassy_executor::task]
pub async fn capture_task(bus: SystemI2cBus, config: Config) {
    let sensor = Bmi270::new(bus);
    let mut revision = 0u32;
    let mut read_errors = 0u32;
    let mut last_orientation = Orientation::default();

    loop {
        publish(
            &mut revision,
            Status::Starting,
            last_orientation,
            read_errors,
        );

        match sensor.initialize().await {
            Ok(()) => {
                ::log::info!(
                    "BMI270 IMU started: sensor={} Hz, fusion={} Hz",
                    DEFAULT_SENSOR_HZ,
                    DEFAULT_FUSION_HZ
                );
            }
            Err(error) => {
                match error {
                    Error::ChipId(id) => ::log::warn!("BMI270 init failed: chip id=0x{:02x}", id),
                    Error::ConfigStatus(status) => {
                        ::log::warn!("BMI270 init failed: config status=0x{:02x}", status)
                    }
                    Error::Bus => ::log::warn!("BMI270 init failed: I2C error"),
                }
                publish(
                    &mut revision,
                    Status::Fault,
                    last_orientation,
                    read_errors,
                );
                Timer::after(INIT_RETRY).await;
                continue;
            }
        }

        let mut fusion = Fusion::new();
        let mut consecutive_errors = 0u8;

        loop {
            Timer::after(config.sample_period).await;

            match sensor.read_sample().await {
                Ok(sample) => {
                    consecutive_errors = 0;
                    last_orientation = fusion.update(
                        sample,
                        config.fusion_dt_seconds,
                        config.fusion_alpha,
                    );
                    publish(
                        &mut revision,
                        Status::Running,
                        last_orientation,
                        read_errors,
                    );
                }
                Err(_) => {
                    read_errors = read_errors.wrapping_add(1);
                    consecutive_errors = consecutive_errors.saturating_add(1);
                    publish(
                        &mut revision,
                        Status::Degraded,
                        last_orientation,
                        read_errors,
                    );

                    if consecutive_errors >= MAX_CONSECUTIVE_READ_ERRORS {
                        ::log::warn!(
                            "BMI270 read failed {} times consecutively; reinitializing",
                            consecutive_errors
                        );
                        publish(
                            &mut revision,
                            Status::Fault,
                            last_orientation,
                            read_errors,
                        );
                        Timer::after(Duration::from_millis(250)).await;
                        break;
                    }
                }
            }
        }
    }
}

const PI: f32 = 3.14159265358979323846;
const RAD_TO_DEG: f32 = 180.0 / PI;

fn radians_to_degrees(value: f32) -> f32 {
    value * RAD_TO_DEG
}

fn clamp_f32(value: f32, min: f32, max: f32) -> f32 {
    if value < min {
        min
    } else if value > max {
        max
    } else {
        value
    }
}

fn wrap_degrees(mut value: f32) -> f32 {
    while value > 180.0 {
        value -= 360.0;
    }
    while value < -180.0 {
        value += 360.0;
    }
    value
}

fn sqrt_approx(value: f32) -> f32 {
    if value <= 0.0 {
        return 0.0;
    }

    let mut estimate = if value > 1.0 { value } else { 1.0 };
    for _ in 0..6 {
        estimate = 0.5 * (estimate + value / estimate);
    }
    estimate
}

/// Fast atan2 approximation suitable for the complementary filter. Maximum
/// error is small compared with the uncalibrated sensor/mounting error of this
/// initial UI service, and it avoids adding a libm dependency to the firmware.
fn atan2_approx(y: f32, x: f32) -> f32 {
    if x == 0.0 && y == 0.0 {
        return 0.0;
    }

    let abs_y = if y < 0.0 { -y } else { y } + 1.0e-10;
    let (ratio, base) = if x < 0.0 {
        ((x + abs_y) / (abs_y - x), 3.0 * PI / 4.0)
    } else {
        ((x - abs_y) / (x + abs_y), PI / 4.0)
    };
    let angle = base + (0.1963 * ratio * ratio - 0.9817) * ratio;

    if y < 0.0 { -angle } else { angle }
}

// BMI270 maximum-FIFO configuration blob from Bosch Sensortec's
// BMI270_SensorAPI v2.86.1, `bmi270_maximum_fifo.c`.
//
// Copyright (c) 2023 Bosch Sensortec GmbH. All rights reserved.
//
// Redistribution and use in source and binary forms, with or without
// modification, are permitted provided that the following conditions are met:
// 1. Redistributions of source code must retain the above copyright notice,
//    this list of conditions and the following disclaimer.
// 2. Redistributions in binary form must reproduce the above copyright notice,
//    this list of conditions and the following disclaimer in the documentation
//    and/or other materials provided with the distribution.
// 3. Neither the name of the copyright holder nor contributors may be used to
//    endorse or promote products derived from this software without specific
//    prior written permission.
//
// THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
// AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
// IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE
// ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE
// LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR
// CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF
// SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS
// INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN
// CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE)
// ARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE
// POSSIBILITY OF SUCH DAMAGE.
const BMI270_MAXIMUM_FIFO_CONFIG: &[u8] = &[
    0xc8, 0x2e, 0x00, 0x2e, 0x80, 0x2e, 0x1a, 0x00, 0xc8, 0x2e, 0x00, 0x2e, 0xc8, 0x2e, 0x00, 0x2e,
    0xc8, 0x2e, 0x00, 0x2e, 0xc8, 0x2e, 0x00, 0x2e, 0xc8, 0x2e, 0x00, 0x2e, 0xc8, 0x2e, 0x00, 0x2e,
    0x90, 0x32, 0x21, 0x2e, 0x59, 0xf5, 0x10, 0x30, 0x21, 0x2e, 0x6a, 0xf5, 0x1a, 0x24, 0x22, 0x00,
    0x80, 0x2e, 0x3b, 0x00, 0xc8, 0x2e, 0x44, 0x47, 0x22, 0x00, 0x37, 0x00, 0xa4, 0x00, 0xff, 0x0f,
    0xd1, 0x00, 0x07, 0xad, 0x80, 0x2e, 0x00, 0xc1, 0x80, 0x2e, 0x00, 0xc1, 0x80, 0x2e, 0x00, 0xc1,
    0x80, 0x2e, 0x00, 0xc1, 0x80, 0x2e, 0x00, 0xc1, 0x80, 0x2e, 0x00, 0xc1, 0x80, 0x2e, 0x00, 0xc1,
    0x80, 0x2e, 0x00, 0xc1, 0x80, 0x2e, 0x00, 0xc1, 0x80, 0x2e, 0x00, 0xc1, 0x80, 0x2e, 0x00, 0xc1,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x11, 0x24, 0xfc, 0xf5, 0x80, 0x30, 0x40, 0x42, 0x50, 0x50,
    0x00, 0x30, 0x12, 0x24, 0xeb, 0x00, 0x03, 0x30, 0x00, 0x2e, 0xc1, 0x86, 0x5a, 0x0e, 0xfb, 0x2f,
    0x21, 0x2e, 0xfc, 0xf5, 0x13, 0x24, 0x63, 0xf5, 0xe0, 0x3c, 0x48, 0x00, 0x22, 0x30, 0xf7, 0x80,
    0xc2, 0x42, 0xe1, 0x7f, 0x3a, 0x25, 0xfc, 0x86, 0xf0, 0x7f, 0x41, 0x33, 0x98, 0x2e, 0xc2, 0xc4,
    0xd6, 0x6f, 0xf1, 0x30, 0xf1, 0x08, 0xc4, 0x6f, 0x11, 0x24, 0xff, 0x03, 0x12, 0x24, 0x00, 0xfc,
    0x61, 0x09, 0xa2, 0x08, 0x36, 0xbe, 0x2a, 0xb9, 0x13, 0x24, 0x38, 0x00, 0x64, 0xbb, 0xd1, 0xbe,
    0x94, 0x0a, 0x71, 0x08, 0xd5, 0x42, 0x21, 0xbd, 0x91, 0xbc, 0xd2, 0x42, 0xc1, 0x42, 0x00, 0xb2,
    0xfe, 0x82, 0x05, 0x2f, 0x50, 0x30, 0x21, 0x2e, 0x21, 0xf2, 0x00, 0x2e, 0x00, 0x2e, 0xd0, 0x2e,
    0xf0, 0x6f, 0x02, 0x30, 0x02, 0x42, 0x20, 0x26, 0xe0, 0x6f, 0x02, 0x31, 0x03, 0x40, 0x9a, 0x0a,
    0x02, 0x42, 0xf0, 0x37, 0x05, 0x2e, 0x5e, 0xf7, 0x10, 0x08, 0x12, 0x24, 0x1e, 0xf2, 0x80, 0x42,
    0x83, 0x84, 0xf1, 0x7f, 0x0a, 0x25, 0x13, 0x30, 0x83, 0x42, 0x3b, 0x82, 0xf0, 0x6f, 0x00, 0x2e,
    0x00, 0x2e, 0xd0, 0x2e, 0x12, 0x40, 0x52, 0x42, 0x00, 0x2e, 0x12, 0x40, 0x52, 0x42, 0x3e, 0x84,
    0x00, 0x40, 0x40, 0x42, 0x7e, 0x82, 0xe1, 0x7f, 0xf2, 0x7f, 0x98, 0x2e, 0x6a, 0xd6, 0x21, 0x30,
    0x23, 0x2e, 0x61, 0xf5, 0xeb, 0x2c, 0xe1, 0x6f,
];
