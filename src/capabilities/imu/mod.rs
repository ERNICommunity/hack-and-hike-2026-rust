//! BMI270 + BMM150 motion/orientation capability.
//!
//! CPU1 owns raw sensor access and fusion. CPU0 receives converted physical
//! sensor measurements alongside the fused orientation/status state. Sensor-chip
//! register formats remain private to the capability.

mod bmi270;
mod bmm150;
mod channels;
mod fusion;
mod magnetic;
mod task;

use embassy_time::Duration;

pub use channels::Imu;
pub(crate) use channels::{Endpoints, Runtime, init_endpoints};
pub(crate) use task::capture_task;

/// Host-side accelerometer/gyroscope acquisition target.
pub(crate) const DEFAULT_SENSOR_HZ: u32 = 100;
/// Fusion runs once per host acquisition.
pub(crate) const DEFAULT_FUSION_HZ: u32 = 100;
/// BMM150 is configured for its maximum 30 Hz normal-mode ODR.
pub(crate) const DEFAULT_MAG_HZ: u32 = 30;

/// Runtime-tunable fusion parameters.
#[derive(Clone, Copy)]
pub(crate) struct Config {
    pub sample_period: Duration,
    pub roll_pitch_alpha: f32,
    pub yaw_alpha: f32,
}

pub(crate) const DEFAULT_CONFIG: Config = Config {
    sample_period: Duration::from_millis(10),
    roll_pitch_alpha: 0.98,
    // Magnetic north is a slow absolute reference. Fusion dynamically reduces
    // its authority during fast motion rather than dropping it completely.
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

#[derive(Clone, Copy, Debug)]
pub struct Orientation {
    /// Euler values are presentation/diagnostic outputs only. They necessarily
    /// have branch singularities and must not be used to reconstruct 3-D pose.
    pub roll_deg: f32,
    pub pitch_deg: f32,
    /// Magnetometer-corrected magnetic heading. No magnetic-declination
    /// correction is applied, so this is magnetic yaw.
    pub yaw_deg: f32,
    /// World gravity (down) expressed in the physical display/screen frame.
    pub gravity_screen: [f32; 3],
    /// Magnetic north expressed in the same screen frame and kept orthogonal to
    /// gravity by fusion. This remains well-defined through Euler poles.
    pub north_screen: [f32; 3],
}

impl Default for Orientation {
    fn default() -> Self {
        Self {
            roll_deg: 0.0,
            pitch_deg: 0.0,
            yaw_deg: 0.0,
            gravity_screen: [0.0, 0.0, 1.0],
            north_screen: [1.0, 0.0, 0.0],
        }
    }
}

/// One coherent latest-value IMU capability sample.
///
/// Acceleration is expressed in m/s², angular velocity in degrees/s, and the
/// optional magnetic vector in microtesla. `None` measurements indicate that no
/// valid reading for that sensor is available in the current acquisition session.
#[derive(Clone, Copy, Debug)]
pub struct Sample {
    pub revision: u32,
    pub acceleration_m_s2: Option<[f32; 3]>,
    pub angular_velocity_deg_s: Option<[f32; 3]>,
    pub magnetic_field_ut: Option<[f32; 3]>,
    pub status: Status,
    pub orientation: Orientation,
    pub mag_status: MagStatus,
    pub mag_field_strength_ut: f32,
    pub mag_calibration_percent: u8,
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Measurements {
    pub(super) acceleration_m_s2: Option<[f32; 3]>,
    pub(super) angular_velocity_deg_s: Option<[f32; 3]>,
    pub(super) magnetic_field_ut: Option<[f32; 3]>,
}
