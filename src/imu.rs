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
    // Magnetic heading is a slow drift correction; gyro remains authoritative
    // for real motion. Applied only on fresh 30 Hz magnetic samples.
    yaw_alpha: 0.98,
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
    pub mag_status: MagStatus,
    pub mag_field_ut: f32,
    pub mag_calibration_percent: u8,
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
// Hard-iron offsets inside the CoreS3 enclosure can approach the BMM150's own
// measurement limits. The sensor is specified around +/-1300 uT on X/Y and
// +/-2500 uT on Z, so a valid 3-D vector can exceed 2000 uT in magnitude. Learn
// those raw offsets first; only the calibrated vector is judged as geomagnetic.
const MAG_LEARNING_MIN_UT: f32 = 5.0;
const MAG_LEARNING_MAX_UT: f32 = 4000.0;
// Recover quickly after good magnetic data returns, but require roughly one
// second of consecutive bad 30 Hz samples before declaring the magnetometer
// disturbed. Short magnitude dips should not make the whole IMU status flap.
const MAG_GOOD_SAMPLES_TO_READY: u8 = 8;
const MAG_BAD_SAMPLES_TO_DISTURBED: u8 = 30;
// A bad magnetic heading must never be able to erase a real turn. At 30 Hz this
// permits at most about six degrees/second of magnetic drift correction.
const MAX_MAG_YAW_CORRECTION_PER_SAMPLE_DEG: f32 = 0.20;
// The BMM150 cannot produce a real 20-degree heading step between adjacent 30 Hz
// frames within the BMI270's +/-500 dps gyro range. Treat larger single-frame
// changes as magnetic glitches instead of allowing them to drag yaw.
const MAX_MAG_HEADING_STEP_DEG: f32 = 20.0;
// Heading is numerically unstable when the leveled field points almost entirely
// vertically. Require a small horizontal component before using atan2.
const MIN_MAG_HORIZONTAL_FIELD_UT: f32 = 2.0;

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

    async fn initialize_bmm150(&self) -> Result<bmm150::Trim, Error> {
        self.write_register(REG_IF_CONF, 0x20).await?;
        self.write_register(REG_PWR_CONF, 0x00).await?;
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
    gravity_body: [f32; 3],
    last_mag_heading: Option<f32>,
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
            gravity_body: [0.0, 0.0, 1.0],
            last_mag_heading: None,
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
        let measured_gravity = normalize3(accel_g);

        if !self.initialized {
            if let Some(gravity) = measured_gravity {
                self.gravity_body = gravity;
            }
            let (roll, pitch) = attitude_from_gravity(self.gravity_body);
            self.orientation.roll_deg = roll;
            self.orientation.pitch_deg = pitch;

            let screen_gravity = normalize3(screen_vector_from_body(self.gravity_body))
                .unwrap_or([0.0, 0.0, 1.0]);
            let initial_heading = magnetic_field_ut
                .map(screen_vector_from_body)
                .and_then(|field| gravity_compensated_heading(field, screen_gravity));
            self.orientation.yaw_deg = initial_heading.unwrap_or(0.0);
            self.last_mag_heading = initial_heading;
            self.initialized = true;
            return self.orientation;
        }

        // Propagate one gravity vector with the complete body-rate vector instead
        // of integrating roll/pitch as independent Euler angles. A yaw rotation
        // around gravity therefore leaves tilt unchanged by construction.
        let predicted_gravity = integrate_gravity(self.gravity_body, gyro_dps, dt_seconds);
        let accel_norm_sq = dot3(accel_g, accel_g);
        let accel_plausible = (0.75 * 0.75..=1.25 * 1.25).contains(&accel_norm_sq);
        let alpha = clamp_f32(roll_pitch_alpha, 0.0, 1.0);
        self.gravity_body = if accel_plausible {
            if let Some(measured) = measured_gravity {
                let blended = [
                    alpha * predicted_gravity[0] + (1.0 - alpha) * measured[0],
                    alpha * predicted_gravity[1] + (1.0 - alpha) * measured[1],
                    alpha * predicted_gravity[2] + (1.0 - alpha) * measured[2],
                ];
                normalize3(blended).unwrap_or(predicted_gravity)
            } else {
                predicted_gravity
            }
        } else {
            predicted_gravity
        };

        let (roll, pitch) = attitude_from_gravity(self.gravity_body);
        self.orientation.roll_deg = roll;
        self.orientation.pitch_deg = pitch;

        let screen_gravity = normalize3(screen_vector_from_body(self.gravity_body))
            .unwrap_or([0.0, 0.0, 1.0]);
        let screen_gyro = screen_vector_from_body(gyro_dps);
        let raw_magnetic_heading = magnetic_field_ut
            .map(screen_vector_from_body)
            .and_then(|field| gravity_compensated_heading(field, screen_gravity));
        let magnetic_heading = raw_magnetic_heading.and_then(|heading| {
            let stable = self
                .last_mag_heading
                .map(|last| abs_f32(wrap_degrees(heading - last)) <= MAX_MAG_HEADING_STEP_DEG)
                .unwrap_or(true);
            if stable {
                self.last_mag_heading = Some(heading);
                Some(heading)
            } else {
                None
            }
        });

        // Yaw rate is the component of angular velocity around local gravity.
        // This is orientation-independent and uses the same filtered gravity
        // estimate that defines roll/pitch, preventing axis leakage.
        let yaw_rate_dps = dot3(screen_gyro, screen_gravity);
        let predicted_yaw = wrap_degrees(self.orientation.yaw_deg + yaw_rate_dps * dt_seconds);

        self.orientation.yaw_deg = if let Some(heading) = magnetic_heading {
            let correction = wrap_degrees(heading - predicted_yaw);
            let requested = (1.0 - clamp_f32(yaw_alpha, 0.0, 1.0)) * correction;
            let applied = clamp_f32(
                requested,
                -MAX_MAG_YAW_CORRECTION_PER_SAMPLE_DEG,
                MAX_MAG_YAW_CORRECTION_PER_SAMPLE_DEG,
            );
            wrap_degrees(predicted_yaw + applied)
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
    mag_status: MagStatus,
    mag_field_ut: f32,
    mag_calibration_percent: u8,
) {
    *revision = revision.wrapping_add(1);
    LATEST.signal(Snapshot {
        revision: *revision,
        status,
        orientation,
        mag_status,
        mag_field_ut,
        mag_calibration_percent,
    });
}

#[embassy_executor::task]
pub async fn capture_task(bus: SystemI2cBus, config: Config) {
    let sensor = Bmi270::new(bus);
    let mut revision = 0u32;
    let mut last_orientation = Orientation::default();

    loop {
        publish(
            &mut revision,
            Status::Starting,
            last_orientation,
            MagStatus::Missing,
            0.0,
            0,
        );

        match sensor.initialize().await {
            Ok(()) => {}
            Err(error) => {
                log_init_error("BMI270", error);
                publish(
                    &mut revision,
                    Status::Fault,
                    last_orientation,
                    MagStatus::Missing,
                    0.0,
                    0,
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
        let mut mag_good_samples = 0u8;
        let mut mag_bad_samples = 0u8;

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
                        mag_good_samples = 0;
                        mag_bad_samples = 0;
                        last_mag_frame = None;
                        last_mag_update = now;
                    }
                    Err(_) => {
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
                    let mut magnetic_for_fusion: Option<[f32; 3]> = None;

                    if let Some(trim) = mag_trim {
                        let is_new_frame = last_mag_frame != Some(sample.mag_data);
                        if is_new_frame {
                            last_mag_frame = Some(sample.mag_data);
                            if let Some(mag) = bmm150::compensate(sample.mag_data, trim) {
                                if mag.data_ready {
                                    last_mag_update = now;

                                    let body_field = [
                                        mag.field_ut[0],
                                        -mag.field_ut[1],
                                        -mag.field_ut[2],
                                    ];
                                    let raw_learnable = (MAG_LEARNING_MIN_UT..=MAG_LEARNING_MAX_UT)
                                        .contains(&mag.field_strength_ut);

                                    // Once calibration is accepted, freeze its extrema.
                                    // Continuously moving min/max values made the heading
                                    // jump whenever a small movement found a new extreme.
                                    let calibration_ready_before = mag_calibration.is_ready();
                                    if !calibration_ready_before && raw_learnable {
                                        mag_calibration.observe(body_field);
                                    }

                                    let calibration_ready = mag_calibration.is_ready();
                                    let corrected_field = mag_calibration.apply(body_field);
                                    mag_field_ut = if calibration_ready {
                                        bmm150::vector_length(corrected_field)
                                    } else {
                                        mag.field_strength_ut
                                    };

                                    if !calibration_ready {
                                        magnetic_for_fusion = None;
                                        mag_good_samples = 0;
                                        if raw_learnable {
                                            mag_bad_samples = 0;
                                            mag_status = MagStatus::Learning;
                                        } else {
                                            mag_bad_samples = mag_bad_samples.saturating_add(1);
                                            if mag_bad_samples >= MAG_BAD_SAMPLES_TO_DISTURBED {
                                                mag_status = MagStatus::Disturbed;
                                            }
                                        }
                                    } else {
                                        let field_good = (bmm150::GOOD_FIELD_MIN_UT
                                            ..=bmm150::GOOD_FIELD_MAX_UT)
                                            .contains(&mag_field_ut);

                                        if field_good {
                                            mag_good_samples = mag_good_samples.saturating_add(1);
                                            mag_bad_samples = 0;
                                            magnetic_for_fusion = Some(corrected_field);
                                            if mag_good_samples >= MAG_GOOD_SAMPLES_TO_READY {
                                                mag_status = MagStatus::Ready;
                                            }
                                        } else {
                                            // Reject the individual bad vector immediately
                                            // from yaw fusion, but do not flap the whole IMU
                                            // to DEGRADED unless the mismatch persists.
                                            mag_bad_samples = mag_bad_samples.saturating_add(1);
                                            mag_good_samples = 0;
                                            if mag_bad_samples >= MAG_BAD_SAMPLES_TO_DISTURBED {
                                                mag_status = MagStatus::Disturbed;
                                            }
                                        }
                                    }
                                }
                            } else {
                                mag_bad_samples = mag_bad_samples.saturating_add(1);
                                mag_good_samples = 0;
                                if mag_bad_samples >= MAG_BAD_SAMPLES_TO_DISTURBED {
                                    mag_status = MagStatus::Disturbed;
                                }
                            }
                        }

                        if now - last_mag_update >= MAG_STALE {
                            mag_status = MagStatus::Missing;
                            mag_good_samples = 0;
                            mag_bad_samples = 0;
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
                        mag_status,
                        mag_field_ut,
                        mag_calibration.progress_percent(),
                    );
                }
                Err(_) => {
                    consecutive_errors = consecutive_errors.saturating_add(1);
                    publish(
                        &mut revision,
                        Status::Degraded,
                        last_orientation,
                        mag_status,
                        mag_field_ut,
                        mag_calibration.progress_percent(),
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
                            mag_status,
                            mag_field_ut,
                            mag_calibration.progress_percent(),
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

fn screen_vector_from_body(value: [f32; 3]) -> [f32; 3] {
    [value[2], -value[0], -value[1]]
}

fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalize3(value: [f32; 3]) -> Option<[f32; 3]> {
    let norm_sq = dot3(value, value);
    if norm_sq < 0.01 {
        return None;
    }
    let inverse = 1.0 / sqrt_approx(norm_sq);
    Some([value[0] * inverse, value[1] * inverse, value[2] * inverse])
}

fn integrate_gravity(gravity: [f32; 3], gyro_dps: [f32; 3], dt_seconds: f32) -> [f32; 3] {
    let omega = [
        gyro_dps[0] * DEG_TO_RAD,
        gyro_dps[1] * DEG_TO_RAD,
        gyro_dps[2] * DEG_TO_RAD,
    ];
    // Coordinates of an inertially fixed gravity vector in a rotating body obey
    // g_dot = -omega x g = g x omega.
    let derivative = cross3(gravity, omega);
    let predicted = [
        gravity[0] + derivative[0] * dt_seconds,
        gravity[1] + derivative[1] * dt_seconds,
        gravity[2] + derivative[2] * dt_seconds,
    ];
    normalize3(predicted).unwrap_or(gravity)
}

fn attitude_from_gravity(gravity: [f32; 3]) -> (f32, f32) {
    let [gx, gy, gz] = gravity;
    let roll = radians_to_degrees(atan2_approx(gy, gz));
    let pitch = radians_to_degrees(atan2_approx(-gx, sqrt_approx(gy * gy + gz * gz)));
    (roll, pitch)
}

/// Level the magnetic vector directly from gravity rather than converting
/// gravity to Euler roll/pitch first. The shortest-arc quaternion maps the
/// measured gravity vector onto +Z, so heading remains well-conditioned when
/// the display is face-up/face-down (where Euler yaw/roll are singular).
fn gravity_compensated_heading(field: [f32; 3], gravity: [f32; 3]) -> Option<f32> {
    let leveled = level_vector_to_gravity(field, gravity);
    let horizontal_sq = leveled[0] * leveled[0] + leveled[1] * leveled[1];
    if horizontal_sq < MIN_MAG_HORIZONTAL_FIELD_UT * MIN_MAG_HORIZONTAL_FIELD_UT {
        return None;
    }
    Some(wrap_degrees(radians_to_degrees(atan2_approx(
        -leveled[1],
        leveled[0],
    ))))
}

fn level_vector_to_gravity(value: [f32; 3], gravity: [f32; 3]) -> [f32; 3] {
    let gz = clamp_f32(gravity[2], -1.0, 1.0);

    // gravity ~= -Z needs an explicit 180-degree leveling axis because the
    // shortest-arc quaternion's scalar/vector terms both approach zero.
    if gz < -0.999 {
        return [value[0], -value[1], -value[2]];
    }

    let mut qw = 1.0 + gz;
    let mut qx = gravity[1];
    let mut qy = -gravity[0];
    let mut qz = 0.0;
    let norm = sqrt_approx(qw * qw + qx * qx + qy * qy + qz * qz);
    if norm <= 0.0001 {
        return value;
    }
    let inverse = 1.0 / norm;
    qw *= inverse;
    qx *= inverse;
    qy *= inverse;
    qz *= inverse;

    rotate_by_quaternion(value, qw, qx, qy, qz)
}

fn rotate_by_quaternion(value: [f32; 3], qw: f32, qx: f32, qy: f32, qz: f32) -> [f32; 3] {
    let tx = 2.0 * (qy * value[2] - qz * value[1]);
    let ty = 2.0 * (qz * value[0] - qx * value[2]);
    let tz = 2.0 * (qx * value[1] - qy * value[0]);

    [
        value[0] + qw * tx + (qy * tz - qz * ty),
        value[1] + qw * ty + (qz * tx - qx * tz),
        value[2] + qw * tz + (qx * ty - qy * tx),
    ]
}

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
