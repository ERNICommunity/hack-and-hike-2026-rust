//! BMI270 + BMM150 orientation capability.
//!
//! CPU1 owns raw sensor access and fusion. CPU0 receives both human-readable
//! Euler angles and the complete gravity/north basis used by 3-D consumers.

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
    // Magnetic north is a slow absolute reference. Fusion dynamically reduces
    // its authority during fast motion rather than dropping it completely.
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

#[derive(Clone, Copy, Debug)]
pub(crate) struct Orientation {
    /// Euler values are presentation/diagnostic outputs only. They necessarily
    /// have branch singularities and must not be used to reconstruct 3-D pose.
    pub(crate) roll_deg: f32,
    pub(crate) pitch_deg: f32,
    /// Magnetometer-corrected magnetic heading. No magnetic-declination
    /// correction is applied, so this is magnetic yaw.
    pub(crate) yaw_deg: f32,
    /// World gravity (down) expressed in the physical display/screen frame.
    pub(crate) gravity_screen: [f32; 3],
    /// Magnetic north expressed in the same screen frame and kept orthogonal to
    /// gravity by fusion. This remains well-defined through Euler poles.
    pub(crate) north_screen: [f32; 3],
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

#[derive(Clone, Copy, Debug)]
pub(crate) struct Snapshot {
    pub(crate) revision: u32,
    pub(crate) status: Status,
    pub(crate) orientation: Orientation,
    pub(crate) mag_status: MagStatus,
    pub(crate) mag_field_ut: f32,
    pub(crate) mag_calibration_percent: u8,
}
