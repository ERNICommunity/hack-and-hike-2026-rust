//! BMI270 + BMM150 orientation service.
//!
//! CPU1 owns raw sensor access and fusion. CPU0 sees only the semantic
//! configuration/status types and the replace-latest [`Input`] endpoint.

mod bmi270;
mod bmm150;
mod channels;
mod fusion;
mod magnetic;
mod task;

use embassy_time::Duration;

pub(crate) use channels::Input;
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
    pub(crate) sample_period: Duration,
    pub(crate) roll_pitch_alpha: f32,
    pub(crate) yaw_alpha: f32,
}

pub(crate) const DEFAULT_CONFIG: Config = Config {
    sample_period: Duration::from_millis(10),
    roll_pitch_alpha: 0.98,
    // Magnetic heading is only a slow/quiet-state absolute reference. Gyro is
    // authoritative during motion.
    yaw_alpha: 0.98,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub(crate) enum Status {
    Starting = 0,
    Running = 1,
    Degraded = 2,
    Fault = 3,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub(crate) enum MagStatus {
    Missing = 0,
    Learning = 1,
    Ready = 2,
    Disturbed = 3,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Orientation {
    pub(crate) roll_deg: f32,
    pub(crate) pitch_deg: f32,
    /// Magnetometer-corrected magnetic heading when BMM150 data is healthy.
    /// No magnetic-declination correction is applied, so this is magnetic yaw.
    pub(crate) yaw_deg: f32,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Snapshot {
    pub(crate) revision: u32,
    pub(crate) status: Status,
    pub(crate) orientation: Orientation,
    pub(crate) mag_status: MagStatus,
    pub(crate) mag_field_ut: f32,
    pub(crate) mag_calibration_percent: u8,
}
