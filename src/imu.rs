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

/// Host-side accelerometer/gyroscope acquisition target.
pub const DEFAULT_SENSOR_HZ: u32 = 100;
/// Fusion runs once per host acquisition.
pub const DEFAULT_FUSION_HZ: u32 = 100;
/// BMI270 gyroscope data registers run faster internally to reduce phase lag
/// during quick turns while the host keeps the proven 100 Hz I2C cadence.
const GYRO_SENSOR_ODR_HZ: u32 = 400;
/// BMM150 is configured for its maximum 30 Hz normal-mode ODR.
pub const DEFAULT_MAG_HZ: u32 = 30;

/// Runtime-tunable fusion parameters.
#[derive(Clone, Copy)]
pub struct Config {
    pub sample_period: Duration,
    pub roll_pitch_alpha: f32,
    pub yaw_alpha: f32,
}

pub const DEFAULT_CONFIG: Config = Config {
    sample_period: Duration::from_millis(10),
    roll_pitch_alpha: 0.98,
    // Magnetic heading is only a slow/quiet-state absolute reference. Gyro is
    // authoritative during motion.
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

// Acceleration stays at 100 Hz. Gyro runs at 400 Hz, performance filtering,
// normal bandwidth: lower group delay and much higher bandwidth during fast yaw
// without increasing host I2C traffic. Range remains +/-2000 dps.
const ACC_CONF_100HZ: u8 = 0xA8;
const GYR_CONF_400HZ: u8 = 0xAA;
const ACC_RANGE_4G: u8 = 0x01;
const GYR_RANGE_2000DPS: u8 = 0x00;
const PWR_CTRL_ACC_GYR: u8 = 0x06;
const PWR_CTRL_ACC_GYR_AUX: u8 = 0x0F;

// Poll the 30 Hz BMM150 at 100 Hz inside BMI270. This does not add host I2C
// traffic, but halves worst-case AUX pickup latency compared with 50 Hz.
const AUX_CONF_100HZ: u8 = 0x48;
const AUX_IF_DATA_MODE_8_BYTES: u8 = 0x4F;
const AUX_IF_MANUAL_MODE: u8 = 0x80;
const AUX_IF_TRIM_2K_PULLUP: u8 = 0x03;

// BMM150 normal mode, 30 Hz ODR, regular preset repetitions.
const BMM_SOFT_RESET_AND_POWER: u8 = 0x83;
const BMM_NORMAL_30HZ: u8 = 0x38;
const BMM_REP_XY_REGULAR: u8 = 0x04;
const BMM_REP_Z_REGULAR: u8 = 0x07;

const ACC_G_PER_LSB: f32 = 4.0 / 32768.0;
const GYR_DPS_PER_LSB: f32 = 2000.0 / 32768.0;
// BMI270 sensor time is a free-running 24-bit counter at exactly 25.6 kHz.
const SENSOR_TIME_TICK_SECONDS: f32 = 1.0 / 25_600.0;
const SENSOR_TIME_MASK: u32 = 0x00FF_FFFF;
const MAX_FUSION_SAMPLE_GAP_TICKS: u32 = 1_280; // 50 ms
const NOMINAL_FUSION_TICKS: u32 = 25_600 / DEFAULT_SENSOR_HZ;

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
// Quiet-state magnetic correction remains deliberately slow for small drift.
const MAX_MAG_YAW_CORRECTION_PER_SAMPLE_DEG: f32 = 0.20;
// Magnetometer observations are not fused while the device is rotating faster
// than this on any axis. The BMM150 is slower and delayed relative to gyro; it
// is much more reliable as an absolute reference after motion settles.
const MAG_FUSION_MAX_RATE_DPS: f32 = 45.0;
// Require fresh low-motion MAG frames before using the compass after any turn.
// At 30 Hz this provides roughly 100 ms for BMM150/AUX pipeline latency to clear.
const MAG_QUIET_SAMPLES_BEFORE_FUSION: u8 = 3;
// Initial/reacquired north must be consistent over multiple independent BMM150
// frames. Compare heading-minus-gyro offsets so small residual motion cancels.
const MAG_INITIAL_LOCK_SAMPLES: u8 = 5;
const MAX_MAG_INITIAL_OFFSET_JITTER_DEG: f32 = 6.0;
// A large but stable discrepancy after real motion is evidence that gyro
// integration lost angle. Reacquire only after a longer consistency proof; a
// stationary magnetic disturbance with no preceding motion remains rejected.
const MAG_RECOVERY_MIN_INNOVATION_DEG: f32 = 30.0;
const MAG_RECOVERY_SAMPLES: u8 = 8;
const MAX_MAG_RECOVERY_OFFSET_JITTER_DEG: f32 = 6.0;
// If a trusted gyro integration is known to have become incomplete (near full
// scale or a long sample gap), explicitly mark absolute yaw untrusted.
const GYRO_NEAR_SATURATION_DPS: f32 = 1950.0;
// The horizontal magnetic component must be measurable, and the device heading
// axis itself must have a meaningful horizontal projection. When the latter is
// nearly vertical, compass heading is physically undefined; gyro yaw bridges
// that region instead of allowing a tilt singularity to flip north.
const MIN_MAG_HORIZONTAL_FIELD_UT: f32 = 2.0;
const MIN_HEADING_AXIS_HORIZONTAL_SQ: f32 = 0.04;

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
    sensor_time: u32,
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
        self.write_register(REG_GYR_CONF, GYR_CONF_400HZ).await?;
        self.write_register(REG_GYR_RANGE, GYR_RANGE_2000DPS).await?;
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

        self.write_register(REG_AUX_CONF, AUX_CONF_100HZ).await?;
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
        // DATA_0..DATA_19 followed by the three SENSORTIME bytes. Bosch defines
        // sensor time as shadowed at the start of a burst that begins in the data
        // registers, so this gives a coherent 25.6 kHz timestamp at no extra I2C
        // transaction cost.
        let mut bytes = [0u8; 23];
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
        let sensor_time = u32::from(bytes[20])
            | (u32::from(bytes[21]) << 8)
            | (u32::from(bytes[22]) << 16);

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
            sensor_time,
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
    previous_yaw_rate_dps: Option<f32>,
    magnetic_heading_locked: bool,
    quiet_mag_samples: u8,
    recovery_armed: bool,
    pending_mag_offset: Option<f32>,
    pending_mag_samples: u8,
    recovery_mag_offset: Option<f32>,
    recovery_mag_samples: u8,
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
            previous_yaw_rate_dps: None,
            magnetic_heading_locked: false,
            quiet_mag_samples: 0,
            recovery_armed: false,
            pending_mag_offset: None,
            pending_mag_samples: 0,
            recovery_mag_offset: None,
            recovery_mag_samples: 0,
            initialized: false,
        }
    }

    fn invalidate_absolute_heading(&mut self) {
        self.magnetic_heading_locked = false;
        self.quiet_mag_samples = 0;
        self.recovery_armed = true;
        self.clear_pending_magnetic_candidate();
        self.clear_recovery_candidate();
    }

    fn reset_rate_history(&mut self) {
        self.previous_yaw_rate_dps = None;
    }

    fn clear_pending_magnetic_candidate(&mut self) {
        self.pending_mag_offset = None;
        self.pending_mag_samples = 0;
    }

    fn clear_recovery_candidate(&mut self) {
        self.recovery_mag_offset = None;
        self.recovery_mag_samples = 0;
    }

    fn note_motion(&mut self) {
        self.quiet_mag_samples = 0;
        self.recovery_armed = true;
        self.clear_pending_magnetic_candidate();
        self.clear_recovery_candidate();
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
            self.orientation.yaw_deg = 0.0;
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

        // Yaw rate is the component of angular velocity around local gravity.
        let yaw_rate_dps = dot3(screen_gyro, screen_gravity);
        // Trapezoidal integration preserves substantially more turn angle during
        // fast acceleration/deceleration than integrating only the newest rate.
        let integrated_yaw_rate = self
            .previous_yaw_rate_dps
            .map(|previous| 0.5 * (previous + yaw_rate_dps))
            .unwrap_or(yaw_rate_dps);
        self.previous_yaw_rate_dps = Some(yaw_rate_dps);
        let predicted_yaw = wrap_degrees(
            self.orientation.yaw_deg + integrated_yaw_rate * dt_seconds,
        );

        let total_rate_dps = max_abs3(gyro_dps);
        if total_rate_dps > MAG_FUSION_MAX_RATE_DPS {
            // Never mix delayed 30 Hz magnetic observations into active motion.
            self.note_motion();
            self.orientation.yaw_deg = predicted_yaw;
            return self.orientation;
        }

        let magnetic_heading = magnetic_field_ut
            .map(screen_vector_from_body)
            .and_then(|field| gravity_compensated_heading(field, screen_gravity));

        self.orientation.yaw_deg = if let Some(heading) = magnetic_heading {
            self.fuse_magnetic_yaw(predicted_yaw, heading, yaw_alpha)
        } else {
            predicted_yaw
        };

        self.orientation
    }

    fn fuse_magnetic_yaw(&mut self, predicted_yaw: f32, heading: f32, yaw_alpha: f32) -> f32 {
        self.quiet_mag_samples = self.quiet_mag_samples.saturating_add(1);
        if self.quiet_mag_samples < MAG_QUIET_SAMPLES_BEFORE_FUSION {
            return predicted_yaw;
        }

        let offset = wrap_degrees(heading - predicted_yaw);

        if !self.magnetic_heading_locked {
            let consistent = self
                .pending_mag_offset
                .map(|previous| {
                    abs_f32(wrap_degrees(offset - previous))
                        <= MAX_MAG_INITIAL_OFFSET_JITTER_DEG
                })
                .unwrap_or(false);

            if consistent {
                self.pending_mag_samples = self.pending_mag_samples.saturating_add(1);
                let previous = self.pending_mag_offset.unwrap_or(offset);
                self.pending_mag_offset = Some(wrap_degrees(
                    previous + 0.25 * wrap_degrees(offset - previous),
                ));
            } else {
                self.pending_mag_offset = Some(offset);
                self.pending_mag_samples = 1;
            }

            if self.pending_mag_samples >= MAG_INITIAL_LOCK_SAMPLES {
                let acquired_offset = self.pending_mag_offset.unwrap_or(offset);
                self.magnetic_heading_locked = true;
                self.recovery_armed = false;
                self.clear_pending_magnetic_candidate();
                self.clear_recovery_candidate();
                return wrap_degrees(predicted_yaw + acquired_offset);
            }

            return predicted_yaw;
        }

        // Large recovery is only legal after actual motion (or explicit timing/
        // saturation invalidation). Once MAG and gyro agree after a turn, disarm
        // it so a later stationary magnetic disturbance cannot redefine north.
        if abs_f32(offset) >= MAG_RECOVERY_MIN_INNOVATION_DEG {
            if !self.recovery_armed {
                self.clear_recovery_candidate();
                return predicted_yaw;
            }

            let consistent = self
                .recovery_mag_offset
                .map(|previous| {
                    abs_f32(wrap_degrees(offset - previous))
                        <= MAX_MAG_RECOVERY_OFFSET_JITTER_DEG
                })
                .unwrap_or(false);

            if consistent {
                self.recovery_mag_samples = self.recovery_mag_samples.saturating_add(1);
                let previous = self.recovery_mag_offset.unwrap_or(offset);
                self.recovery_mag_offset = Some(wrap_degrees(
                    previous + 0.25 * wrap_degrees(offset - previous),
                ));
            } else {
                self.recovery_mag_offset = Some(offset);
                self.recovery_mag_samples = 1;
            }

            if self.recovery_mag_samples >= MAG_RECOVERY_SAMPLES {
                let recovered_offset = self.recovery_mag_offset.unwrap_or(offset);
                self.recovery_armed = false;
                self.clear_recovery_candidate();
                return wrap_degrees(predicted_yaw + recovered_offset);
            }

            return predicted_yaw;
        }

        self.recovery_armed = false;
        self.clear_recovery_candidate();
        let requested = (1.0 - clamp_f32(yaw_alpha, 0.0, 1.0)) * offset;
        let applied = clamp_f32(
            requested,
            -MAX_MAG_YAW_CORRECTION_PER_SAMPLE_DEG,
            MAX_MAG_YAW_CORRECTION_PER_SAMPLE_DEG,
        );
        wrap_degrees(predicted_yaw + applied)
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
                    "BMI270+BMM150 IMU started: fusion={} Hz, gyro={} Hz, mag={} Hz",
                    DEFAULT_FUSION_HZ,
                    GYRO_SENSOR_ODR_HZ,
                    DEFAULT_MAG_HZ
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
        let mut last_sensor_time: Option<u32> = None;
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

                    let delta_ticks = last_sensor_time
                        .map(|previous| sample.sensor_time.wrapping_sub(previous) & SENSOR_TIME_MASK)
                        .unwrap_or(NOMINAL_FUSION_TICKS);
                    last_sensor_time = Some(sample.sensor_time);
                    let timing_gap = delta_ticks == 0 || delta_ticks > MAX_FUSION_SAMPLE_GAP_TICKS;
                    let integration_ticks = if timing_gap {
                        NOMINAL_FUSION_TICKS
                    } else {
                        delta_ticks
                    };
                    let dt_seconds = integration_ticks as f32 * SENSOR_TIME_TICK_SECONDS;

                    // Saturation or a timing discontinuity can lose turn angle.
                    // Mark absolute yaw untrusted, but retain the best gyro path;
                    // quiet-state MAG recovery below will establish north again.
                    if timing_gap || max_abs3(sample.gyro_dps) >= GYRO_NEAR_SATURATION_DPS {
                        fusion.invalidate_absolute_heading();
                    }
                    if timing_gap {
                        fusion.reset_rate_history();
                    }

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

                                    // Once calibration is accepted, freeze its model.
                                    // The calibrator itself balances 3-D coverage and
                                    // validates a candidate on fresh measurements.
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
                                            // A newly calibrated or recovered magnetometer
                                            // must prove several consecutive good vectors
                                            // before it is allowed to define magnetic north.
                                            if mag_status == MagStatus::Ready
                                                || mag_good_samples >= MAG_GOOD_SAMPLES_TO_READY
                                            {
                                                mag_status = MagStatus::Ready;
                                                magnetic_for_fusion = Some(corrected_field);
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

/// Compute magnetic heading without inventing a leveling rotation.
///
/// Project both magnetic north and the fixed screen +X heading axis onto the
/// plane perpendicular to gravity, then measure their signed angle around
/// gravity. This is tilt-invariant wherever heading is physically defined.
fn gravity_compensated_heading(field: [f32; 3], gravity: [f32; 3]) -> Option<f32> {
    let field_along_gravity = dot3(field, gravity);
    let horizontal_field = [
        field[0] - gravity[0] * field_along_gravity,
        field[1] - gravity[1] * field_along_gravity,
        field[2] - gravity[2] * field_along_gravity,
    ];
    let horizontal_field_sq = dot3(horizontal_field, horizontal_field);
    if horizontal_field_sq < MIN_MAG_HORIZONTAL_FIELD_UT * MIN_MAG_HORIZONTAL_FIELD_UT {
        return None;
    }

    let forward_along_gravity = gravity[0];
    let horizontal_forward = [
        1.0 - gravity[0] * forward_along_gravity,
        -gravity[1] * forward_along_gravity,
        -gravity[2] * forward_along_gravity,
    ];
    if dot3(horizontal_forward, horizontal_forward) < MIN_HEADING_AXIS_HORIZONTAL_SQ {
        return None;
    }

    let sine = -dot3(gravity, cross3(horizontal_forward, horizontal_field));
    let cosine = dot3(horizontal_forward, horizontal_field);
    Some(wrap_degrees(radians_to_degrees(atan2_approx(sine, cosine))))
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
