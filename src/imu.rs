//! CPU1 BMI270 + BMM150 acquisition and 9-axis orientation fusion.
//!
//! The BMI270 shares the runtime system-I2C bus with touch and owns the BMM150
//! through its auxiliary I²C sensor hub. Raw hardware access therefore remains
//! entirely on CPU1. CPU0 receives only [`Snapshot`], a replace-latest fused
//! orientation/health value sampled at the presentation rate.

mod bmi270_config;
mod bmm150;

use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use embassy_time::{Duration, Instant, Timer};

use crate::system_i2c::SystemI2cBus;

/// Accelerometer/gyroscope acquisition target.
pub const DEFAULT_SENSOR_HZ: u32 = 100;
/// Fusion runs once per acquired accelerometer/gyroscope sample.
pub const DEFAULT_FUSION_HZ: u32 = 100;
/// BMM150 is configured for its maximum 30 Hz normal-mode ODR.
pub const DEFAULT_MAG_HZ: u32 = 30;

/// Runtime-tunable fusion parameters.
///
/// Actual integration `dt` is measured from `Instant` on every sample rather
/// than assumed from the nominal period, making heading less sensitive to
/// shared-I²C/task scheduling jitter.
#[derive(Clone, Copy)]
pub struct Config {
    pub sample_period: Duration,
    pub roll_pitch_alpha: f32,
    pub yaw_alpha: f32,
}

pub const DEFAULT_CONFIG: Config = Config {
    sample_period: Duration::from_millis(10),
    roll_pitch_alpha: 0.98,
    // Applied only when a fresh 30 Hz magnetic sample is available.
    yaw_alpha: 0.90,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum Status {
    Starting = 0,
    Running = 1,
    Degraded = 2,
    Fault = 3,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum MagStatus {
    Missing = 0,
    Learning = 1,
    Ready = 2,
    Disturbed = 3,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Orientation {
    pub roll_deg: f32,
    pub pitch_deg: f32,
    /// Magnetometer-corrected magnetic heading when BMM150 data is healthy.
    /// No magnetic-declination correction is applied, so this is magnetic yaw.
    pub yaw_deg: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct Snapshot {
    pub revision: u32,
    pub status: Status,
    pub orientation: Orientation,
    pub read_errors: u32,
    pub mag_errors: u32,
    pub mag_status: MagStatus,
    pub mag_field_ut: f32,
    pub mag_calibration_percent: u8,
    pub gyro_bias_ready: bool,
}

static LATEST: Signal<CriticalSectionRawMutex, Snapshot> = Signal::new();

/// Take the newest orientation/status snapshot, if CPU1 published one since the
/// previous take. Multiple CPU1 updates collapse to one latest value.
pub fn take_latest() -> Option<Snapshot> {
    LATEST.try_take()
}

const BMI270_ADDR: u8 = 0x69;
const BMI270_CHIP_ID: u8 = 0x24;
const BMM150_ADDR: u8 = 0x10;
const BMM150_CHIP_ID: u8 = 0x32;

const REG_CHIP_ID: u8 = 0x00;
const REG_STATUS: u8 = 0x03;
const REG_AUX_X_LSB: u8 = 0x04;
const REG_INTERNAL_STATUS: u8 = 0x21;
const REG_ACC_CONF: u8 = 0x40;
const REG_ACC_RANGE: u8 = 0x41;
const REG_GYR_CONF: u8 = 0x42;
const REG_GYR_RANGE: u8 = 0x43;
const REG_AUX_CONF: u8 = 0x44;
const REG_AUX_DEV_ID: u8 = 0x4B;
const REG_AUX_IF_CONF: u8 = 0x4C;
const REG_AUX_RD_ADDR: u8 = 0x4D;
const REG_AUX_WR_ADDR: u8 = 0x4E;
const REG_AUX_WR_DATA: u8 = 0x4F;
const REG_INIT_CTRL: u8 = 0x59;
const REG_INIT_ADDR_0: u8 = 0x5B;
const REG_INIT_DATA: u8 = 0x5E;
const REG_AUX_IF_TRIM: u8 = 0x68;
const REG_IF_CONF: u8 = 0x6B;
const REG_PWR_CONF: u8 = 0x7C;
const REG_PWR_CTRL: u8 = 0x7D;
const REG_CMD: u8 = 0x7E;

const BMM_REG_CHIP_ID: u8 = 0x40;
const BMM_REG_DATA_X_LSB: u8 = 0x42;
const BMM_REG_POWER_CONTROL: u8 = 0x4B;
const BMM_REG_OP_MODE: u8 = 0x4C;
const BMM_REG_REP_XY: u8 = 0x51;
const BMM_REG_REP_Z: u8 = 0x52;
const BMM_DIG_X1: u8 = 0x5D;
const BMM_DIG_Z4_LSB: u8 = 0x62;
const BMM_DIG_Z2_LSB: u8 = 0x68;

const CMD_SOFT_RESET: u8 = 0xB6;
const CONFIG_LOAD_OK: u8 = 0x01;
const AUX_BUSY: u8 = 1 << 2;

// 100 Hz, performance filter, normal bandwidth/averaging.
const ACC_CONF_100HZ: u8 = 0xA8;
const GYR_CONF_100HZ: u8 = 0xA8;
const ACC_RANGE_4G: u8 = 0x01;
const GYR_RANGE_500DPS: u8 = 0x02;
const PWR_CTRL_ACC_GYR: u8 = 0x06;
const PWR_CTRL_ACC_GYR_AUX: u8 = 0x0F;

// BMI270 AUX configuration: 50 Hz sensor-hub polling, 8-byte automatic burst.
// The BMM150 itself produces new data at 30 Hz.
const AUX_CONF_50HZ: u8 = 0x47;
const AUX_IF_DATA_MODE_8_BYTES: u8 = 0x4F;
const AUX_IF_MANUAL_MODE: u8 = 0x80;
const AUX_IF_TRIM_2K_PULLUP: u8 = 0x03;

// BMM150 normal mode, 30 Hz ODR, regular preset repetitions.
const BMM_SOFT_RESET_AND_POWER: u8 = 0x83;
const BMM_NORMAL_30HZ: u8 = 0x38;
const BMM_REP_XY_REGULAR: u8 = 0x04;
const BMM_REP_Z_REGULAR: u8 = 0x07;

const ACC_G_PER_LSB: f32 = 4.0 / 32768.0;
const GYR_DPS_PER_LSB: f32 = 500.0 / 32768.0;
const INIT_RETRY: Duration = Duration::from_secs(1);
const MAG_RETRY: Duration = Duration::from_secs(5);
const MAG_STALE: Duration = Duration::from_secs(1);
const SENSOR_STARTUP: Duration = Duration::from_millis(50);
const MAX_CONSECUTIVE_READ_ERRORS: u8 = 10;

#[derive(Clone, Copy, Debug)]
enum Error {
    Bus,
    ChipId(u8),
    ConfigStatus(u8),
    AuxBusy,
    BmmChipId(u8),
}

#[derive(Clone, Copy)]
struct RawSample {
    accel_g: [f32; 3],
    gyro_dps: [f32; 3],
    mag_data: [u8; 8],
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
        for (offset, chunk) in bmi270_config::MAXIMUM_FIFO_CONFIG.chunks(32).enumerate() {
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
            // another CPU1 system-I2C client cannot interleave them.
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
        if chip_id != BMI270_CHIP_ID {
            return Err(Error::ChipId(chip_id));
        }

        self.write_register(REG_CMD, CMD_SOFT_RESET).await?;
        Timer::after(Duration::from_millis(2)).await;

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

    async fn wait_aux_idle(&self) -> Result<(), Error> {
        for _ in 0..8 {
            if self.read_register(REG_STATUS).await? & AUX_BUSY == 0 {
                return Ok(());
            }
            Timer::after(Duration::from_millis(1)).await;
        }
        Err(Error::AuxBusy)
    }

    async fn aux_write_register(&self, register: u8, value: u8) -> Result<(), Error> {
        self.write_register(REG_AUX_WR_DATA, value).await?;
        self.write_register(REG_AUX_WR_ADDR, register).await?;
        self.wait_aux_idle().await
    }

    async fn aux_read_register(&self, register: u8) -> Result<u8, Error> {
        self.write_register(REG_AUX_IF_CONF, AUX_IF_MANUAL_MODE).await?;
        self.write_register(REG_AUX_RD_ADDR, register).await?;
        self.wait_aux_idle().await?;
        self.read_register(REG_AUX_X_LSB).await
    }

    async fn aux_read_array<const N: usize>(&self, first: u8) -> Result<[u8; N], Error> {
        let mut result = [0u8; N];
        let mut register = first;
        for byte in &mut result {
            *byte = self.aux_read_register(register).await?;
            register = register.wrapping_add(1);
        }
        Ok(result)
    }

    /// Configure the BMM150 through BMI270 setup mode and return its factory
    /// trim. If this fails, accel/gyro remain usable and the caller can retry.
    async fn initialize_bmm150(&self) -> Result<bmm150::Trim, Error> {
        self.write_register(REG_IF_CONF, 0x20).await?;
        self.write_register(REG_PWR_CONF, 0x00).await?;
        // Setup mode requires AUX disabled while manual transactions are issued.
        self.write_register(REG_PWR_CTRL, PWR_CTRL_ACC_GYR).await?;
        self.write_register(REG_AUX_IF_TRIM, AUX_IF_TRIM_2K_PULLUP).await?;
        self.write_register(REG_AUX_IF_CONF, AUX_IF_MANUAL_MODE).await?;
        self.write_register(REG_AUX_DEV_ID, BMM150_ADDR << 1).await?;

        self.aux_write_register(BMM_REG_POWER_CONTROL, BMM_SOFT_RESET_AND_POWER)
            .await?;
        Timer::after(Duration::from_millis(5)).await;

        let chip_id = self.aux_read_register(BMM_REG_CHIP_ID).await?;
        if chip_id != BMM150_CHIP_ID {
            return Err(Error::BmmChipId(chip_id));
        }

        let x1_y1 = self.aux_read_array::<2>(BMM_DIG_X1).await?;
        let z4_x2_y2 = self.aux_read_array::<4>(BMM_DIG_Z4_LSB).await?;
        let z2_to_xy1 = self.aux_read_array::<10>(BMM_DIG_Z2_LSB).await?;
        let trim = bmm150::Trim::from_registers(x1_y1, z4_x2_y2, z2_to_xy1);

        self.aux_write_register(BMM_REG_REP_XY, BMM_REP_XY_REGULAR)
            .await?;
        self.aux_write_register(BMM_REG_REP_Z, BMM_REP_Z_REGULAR)
            .await?;
        self.aux_write_register(BMM_REG_OP_MODE, BMM_NORMAL_30HZ)
            .await?;

        // Switch BMI270 from manual AUX setup to continuous data mode.
        self.write_register(REG_AUX_CONF, AUX_CONF_50HZ).await?;
        self.write_register(REG_AUX_IF_CONF, AUX_IF_DATA_MODE_8_BYTES)
            .await?;
        self.write_register(REG_AUX_RD_ADDR, BMM_REG_DATA_X_LSB).await?;
        self.write_register(REG_PWR_CTRL, PWR_CTRL_ACC_GYR_AUX)
            .await?;
        Timer::after(Duration::from_millis(10)).await;

        Ok(trim)
    }

    async fn disable_aux(&self) {
        let _ = self.write_register(REG_PWR_CTRL, PWR_CTRL_ACC_GYR).await;
    }

    /// One primary-I²C burst reads the BMI270's cached 8-byte auxiliary frame
    /// plus the current accelerometer and gyroscope values.
    async fn read_sample(&self) -> Result<RawSample, Error> {
        let mut bytes = [0u8; 20];
        let mut i2c = self.bus.lock().await;
        i2c.write_read_async(BMI270_ADDR, &[REG_AUX_X_LSB], &mut bytes)
            .await
            .map_err(|_| Error::Bus)?;
        drop(i2c);

        let mut mag_data = [0u8; 8];
        mag_data.copy_from_slice(&bytes[..8]);

        let acc = [
            i16::from_le_bytes([bytes[8], bytes[9]]),
            i16::from_le_bytes([bytes[10], bytes[11]]),
            i16::from_le_bytes([bytes[12], bytes[13]]),
        ];
        let gyr = [
            i16::from_le_bytes([bytes[14], bytes[15]]),
            i16::from_le_bytes([bytes[16], bytes[17]]),
            i16::from_le_bytes([bytes[18], bytes[19]]),
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
            mag_data,
        })
    }
}

#[derive(Clone, Copy)]
struct GyroBias {
    bias_dps: [f32; 3],
    stationary_samples: u16,
    ready: bool,
}

impl GyroBias {
    const fn new() -> Self {
        Self {
            bias_dps: [0.0; 3],
            stationary_samples: 0,
            ready: false,
        }
    }

    fn correct(&mut self, accel_g: [f32; 3], gyro_dps: [f32; 3]) -> [f32; 3] {
        let accel_norm_sq = accel_g[0] * accel_g[0]
            + accel_g[1] * accel_g[1]
            + accel_g[2] * accel_g[2];
        let gyro_max = max_abs3(gyro_dps);
        let stationary = (0.90 * 0.90..=1.10 * 1.10).contains(&accel_norm_sq) && gyro_max < 3.0;

        if stationary {
            self.stationary_samples = self.stationary_samples.saturating_add(1);
            let learn = if self.ready { 0.002 } else { 0.02 };
            for axis in 0..3 {
                self.bias_dps[axis] += (gyro_dps[axis] - self.bias_dps[axis]) * learn;
            }
            if self.stationary_samples >= 100 {
                self.ready = true;
            }
        } else {
            self.stationary_samples = 0;
        }

        [
            gyro_dps[0] - self.bias_dps[0],
            gyro_dps[1] - self.bias_dps[1],
            gyro_dps[2] - self.bias_dps[2],
        ]
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

    fn update(
        &mut self,
        accel_g: [f32; 3],
        gyro_dps: [f32; 3],
        dt_seconds: f32,
        roll_pitch_alpha: f32,
        magnetic_field_ut: Option<[f32; 3]>,
        yaw_alpha: f32,
    ) -> Orientation {
        let [ax, ay, az] = accel_g;
        let [gx, gy, gz] = gyro_dps;

        let acc_roll = radians_to_degrees(atan2_approx(ay, az));
        let acc_pitch = radians_to_degrees(atan2_approx(-ax, sqrt_approx(ay * ay + az * az)));

        if !self.initialized {
            self.orientation.roll_deg = acc_roll;
            self.orientation.pitch_deg = acc_pitch;
            self.orientation.yaw_deg = magnetic_field_ut
                .map(|field| tilt_compensated_heading(field, acc_roll, acc_pitch))
                .unwrap_or(0.0);
            self.initialized = true;
            return self.orientation;
        }

        let alpha = clamp_f32(roll_pitch_alpha, 0.0, 1.0);
        let accel_weight = 1.0 - alpha;
        self.orientation.roll_deg = alpha * (self.orientation.roll_deg + gx * dt_seconds)
            + accel_weight * acc_roll;
        self.orientation.pitch_deg = alpha * (self.orientation.pitch_deg + gy * dt_seconds)
            + accel_weight * acc_pitch;

        let predicted_yaw = wrap_degrees(self.orientation.yaw_deg + gz * dt_seconds);
        self.orientation.yaw_deg = if let Some(field) = magnetic_field_ut {
            let heading = tilt_compensated_heading(
                field,
                self.orientation.roll_deg,
                self.orientation.pitch_deg,
            );
            let correction = wrap_degrees(heading - predicted_yaw);
            wrap_degrees(predicted_yaw + (1.0 - clamp_f32(yaw_alpha, 0.0, 1.0)) * correction)
        } else {
            predicted_yaw
        };

        self.orientation
    }
}

fn publish(
    revision: &mut u32,
    status: Status,
    orientation: Orientation,
    read_errors: u32,
    mag_errors: u32,
    mag_status: MagStatus,
    mag_field_ut: f32,
    mag_calibration_percent: u8,
    gyro_bias_ready: bool,
) {
    *revision = revision.wrapping_add(1);
    LATEST.signal(Snapshot {
        revision: *revision,
        status,
        orientation,
        read_errors,
        mag_errors,
        mag_status,
        mag_field_ut,
        mag_calibration_percent,
        gyro_bias_ready,
    });
}

/// CPU1 acquisition/fusion task.
///
/// Accel/gyro and fusion run at ~100 Hz. BMM150 produces 30 Hz magnetometer
/// samples through the BMI270 sensor hub. CPU0 consumes only latest fused state,
/// normally at 25 Hz, so no raw sensor stream crosses cores.
#[embassy_executor::task]
pub async fn capture_task(bus: SystemI2cBus, config: Config) {
    let sensor = Bmi270::new(bus);
    let mut revision = 0u32;
    let mut read_errors = 0u32;
    let mut mag_errors = 0u32;
    let mut last_orientation = Orientation::default();

    loop {
        publish(
            &mut revision,
            Status::Starting,
            last_orientation,
            read_errors,
            mag_errors,
            MagStatus::Missing,
            0.0,
            0,
            false,
        );

        match sensor.initialize().await {
            Ok(()) => {}
            Err(error) => {
                log_init_error("BMI270", error);
                publish(
                    &mut revision,
                    Status::Fault,
                    last_orientation,
                    read_errors,
                    mag_errors,
                    MagStatus::Missing,
                    0.0,
                    0,
                    false,
                );
                Timer::after(INIT_RETRY).await;
                continue;
            }
        }

        let mut mag_trim = match sensor.initialize_bmm150().await {
            Ok(trim) => {
                ::log::info!(
                    "BMI270+BMM150 IMU started: accel/gyro={} Hz, mag={} Hz, fusion={} Hz",
                    DEFAULT_SENSOR_HZ,
                    DEFAULT_MAG_HZ,
                    DEFAULT_FUSION_HZ
                );
                Some(trim)
            }
            Err(error) => {
                mag_errors = mag_errors.wrapping_add(1);
                log_init_error("BMM150", error);
                sensor.disable_aux().await;
                ::log::warn!("IMU continuing in 6-axis fallback; BMM150 will retry");
                None
            }
        };

        let mut fusion = Fusion::new();
        let mut gyro_bias = GyroBias::new();
        let mut mag_calibration = bmm150::Calibration::new();
        let mut mag_status = if mag_trim.is_some() {
            MagStatus::Learning
        } else {
            MagStatus::Missing
        };
        let mut mag_field_ut = 0.0f32;
        let mut last_mag_frame: Option<[u8; 8]> = None;
        let mut last_mag_update = Instant::now();
        let mut last_mag_retry = Instant::now();
        let mut last_sample_time = Instant::now();
        let mut consecutive_errors = 0u8;

        loop {
            Timer::after(config.sample_period).await;
            let now = Instant::now();

            if mag_trim.is_none() && now - last_mag_retry >= MAG_RETRY {
                last_mag_retry = now;
                match sensor.initialize_bmm150().await {
                    Ok(trim) => {
                        ::log::info!("BMM150 recovered; 9-axis heading fusion enabled");
                        mag_trim = Some(trim);
                        mag_calibration = bmm150::Calibration::new();
                        mag_status = MagStatus::Learning;
                        last_mag_frame = None;
                        last_mag_update = now;
                    }
                    Err(_) => {
                        mag_errors = mag_errors.wrapping_add(1);
                        sensor.disable_aux().await;
                    }
                }
            }

            match sensor.read_sample().await {
                Ok(sample) => {
                    consecutive_errors = 0;
                    let elapsed_ms = (now - last_sample_time).as_millis().clamp(2, 50);
                    last_sample_time = now;
                    let dt_seconds = elapsed_ms as f32 * 0.001;

                    let corrected_gyro = gyro_bias.correct(sample.accel_g, sample.gyro_dps);
                    // Yaw correction is intentionally single-shot per fresh 30 Hz
                    // BMM150 frame; the 100 Hz fusion ticks between frames are gyro-only.
                    let mut magnetic_for_fusion: Option<[f32; 3]> = None;

                    if let Some(trim) = mag_trim {
                        let is_new_frame = last_mag_frame != Some(sample.mag_data);
                        if is_new_frame {
                            last_mag_frame = Some(sample.mag_data);
                            if let Some(mag) = bmm150::compensate(sample.mag_data, trim) {
                                if mag.data_ready {
                                    last_mag_update = now;

                                    // CoreS3 BMI270+BMM150 mounting: M5Unified
                                    // maps magnetometer Y and Z with inverted sign
                                    // to align it with the accel/gyro body frame.
                                    let body_field = [
                                        mag.field_ut[0],
                                        -mag.field_ut[1],
                                        -mag.field_ut[2],
                                    ];
                                    // Once calibrated, do not let an abnormal external
                                    // magnetic field move the learned extrema.
                                    if !mag_calibration.is_ready()
                                        || (bmm150::GOOD_FIELD_MIN_UT..=bmm150::GOOD_FIELD_MAX_UT)
                                            .contains(&mag.field_strength_ut)
                                    {
                                        mag_calibration.observe(body_field);
                                    }
                                    let corrected_field = mag_calibration.apply(body_field);
                                    mag_field_ut = bmm150::vector_length(corrected_field);

                                    let plausible = if mag_calibration.is_ready() {
                                        (bmm150::GOOD_FIELD_MIN_UT..=bmm150::GOOD_FIELD_MAX_UT)
                                            .contains(&mag_field_ut)
                                    } else {
                                        (5.0..=150.0).contains(&mag_field_ut)
                                    };

                                    if plausible {
                                        mag_status = if mag_calibration.is_ready() {
                                            MagStatus::Ready
                                        } else {
                                            MagStatus::Learning
                                        };
                                        magnetic_for_fusion = Some(corrected_field);
                                    } else {
                                        mag_status = MagStatus::Disturbed;
                                        magnetic_for_fusion = None;
                                    }
                                }
                            } else {
                                mag_errors = mag_errors.wrapping_add(1);
                                mag_status = MagStatus::Disturbed;
                                magnetic_for_fusion = None;
                            }
                        }

                        if now - last_mag_update >= MAG_STALE {
                            mag_status = MagStatus::Missing;
                            magnetic_for_fusion = None;
                        }
                    } else {
                        mag_status = MagStatus::Missing;
                        magnetic_for_fusion = None;
                    }

                    last_orientation = fusion.update(
                        sample.accel_g,
                        corrected_gyro,
                        dt_seconds,
                        config.roll_pitch_alpha,
                        magnetic_for_fusion,
                        config.yaw_alpha,
                    );

                    let status = match mag_status {
                        MagStatus::Missing | MagStatus::Disturbed => Status::Degraded,
                        MagStatus::Learning | MagStatus::Ready => Status::Running,
                    };
                    publish(
                        &mut revision,
                        status,
                        last_orientation,
                        read_errors,
                        mag_errors,
                        mag_status,
                        mag_field_ut,
                        mag_calibration.progress_percent(),
                        gyro_bias.ready,
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
                        mag_errors,
                        mag_status,
                        mag_field_ut,
                        mag_calibration.progress_percent(),
                        gyro_bias.ready,
                    );

                    if consecutive_errors >= MAX_CONSECUTIVE_READ_ERRORS {
                        ::log::warn!(
                            "BMI270 read failed {} times consecutively; reinitializing IMU",
                            consecutive_errors
                        );
                        publish(
                            &mut revision,
                            Status::Fault,
                            last_orientation,
                            read_errors,
                            mag_errors,
                            mag_status,
                            mag_field_ut,
                            mag_calibration.progress_percent(),
                            gyro_bias.ready,
                        );
                        Timer::after(Duration::from_millis(250)).await;
                        break;
                    }
                }
            }
        }
    }
}

fn log_init_error(device: &str, error: Error) {
    match error {
        Error::ChipId(id) => ::log::warn!("{} init failed: chip id=0x{:02x}", device, id),
        Error::BmmChipId(id) => ::log::warn!("{} init failed: chip id=0x{:02x}", device, id),
        Error::ConfigStatus(status) => {
            ::log::warn!("{} init failed: config status=0x{:02x}", device, status)
        }
        Error::AuxBusy => ::log::warn!("{} init failed: BMI270 AUX interface busy", device),
        Error::Bus => ::log::warn!("{} init failed: I2C error", device),
    }
}

const PI: f32 = 3.14159265358979323846;
const RAD_TO_DEG: f32 = 180.0 / PI;
const DEG_TO_RAD: f32 = PI / 180.0;

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

fn max_abs3(value: [f32; 3]) -> f32 {
    let a = abs_f32(value[0]);
    let b = abs_f32(value[1]);
    let c = abs_f32(value[2]);
    if a > b {
        if a > c { a } else { c }
    } else if b > c {
        b
    } else {
        c
    }
}

fn abs_f32(value: f32) -> f32 {
    if value < 0.0 { -value } else { value }
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

fn wrap_radians(mut value: f32) -> f32 {
    while value > PI {
        value -= 2.0 * PI;
    }
    while value < -PI {
        value += 2.0 * PI;
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

/// Fast atan2 approximation suitable for attitude/heading fusion without a
/// libm dependency.
fn atan2_approx(y: f32, x: f32) -> f32 {
    if x == 0.0 && y == 0.0 {
        return 0.0;
    }

    let abs_y = abs_f32(y) + 1.0e-10;
    let (ratio, base) = if x < 0.0 {
        ((x + abs_y) / (abs_y - x), 3.0 * PI / 4.0)
    } else {
        ((x - abs_y) / (x + abs_y), PI / 4.0)
    };
    let angle = base + (0.1963 * ratio * ratio - 0.9817) * ratio;

    if y < 0.0 { -angle } else { angle }
}

fn sin_approx(value: f32) -> f32 {
    let mut x = wrap_radians(value);
    if x > PI * 0.5 {
        x = PI - x;
    } else if x < -PI * 0.5 {
        x = -PI - x;
    }

    let x2 = x * x;
    x * (1.0 - x2 / 6.0 + x2 * x2 / 120.0 - x2 * x2 * x2 / 5040.0)
}

fn cos_approx(value: f32) -> f32 {
    sin_approx(value + PI * 0.5)
}

fn tilt_compensated_heading(field_ut: [f32; 3], roll_deg: f32, pitch_deg: f32) -> f32 {
    let roll = roll_deg * DEG_TO_RAD;
    let pitch = pitch_deg * DEG_TO_RAD;
    let sin_roll = sin_approx(roll);
    let cos_roll = cos_approx(roll);
    let sin_pitch = sin_approx(pitch);
    let cos_pitch = cos_approx(pitch);

    let [mx, my, mz] = field_ut;
    let horizontal_x = mx * cos_pitch + mz * sin_pitch;
    let horizontal_y = mx * sin_roll * sin_pitch + my * cos_roll - mz * sin_roll * cos_pitch;

    wrap_degrees(radians_to_degrees(atan2_approx(-horizontal_y, horizontal_x)))
}
