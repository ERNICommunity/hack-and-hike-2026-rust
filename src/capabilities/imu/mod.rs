//! Motion sensing: BMI270 accelerometer/gyroscope plus BMM150 magnetometer.
//!
//! CPU1 reads the sensors, learns the gyroscope bias and the magnetometer's
//! enclosure distortion, and fuses everything into an orientation. CPU0
//! receives [`Sample`]s through the [`Imu`] handle. Sensor registers stay
//! private to this module; the math lives in `hack_and_hike_core::imu`, where
//! it is unit-tested on the host.

mod bmi270;
mod channels;
mod magnetic;
mod task;

pub use channels::Imu;
pub(crate) use channels::{Endpoints, Runtime, endpoints};
pub use hack_and_hike_core::imu::Orientation;
pub(crate) use task::capture_task;

/// Health of the accelerometer/gyroscope acquisition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// The sensor is being initialized; no measurement yet.
    Starting,
    /// Samples are flowing.
    Running,
    /// A read failed; the last orientation is being repeated while retrying.
    Degraded,
    /// Repeated failures; the sensor is being re-initialized.
    Fault,
}

/// Health of the magnetometer, which the heading depends on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MagStatus {
    /// No magnetometer is answering; heading drifts with the gyroscope.
    Missing,
    /// Calibration is collecting samples; move the device in all directions.
    Learning,
    /// Calibrated and delivering plausible fields.
    Ready,
    /// The field does not look like the Earth's; heading is not corrected.
    Disturbed,
}

/// One IMU sample as published to the application.
///
/// Acceleration is in m/s², angular velocity in degrees/s, the magnetic field
/// in microtesla. A `None` measurement means that sensor delivered nothing
/// valid in the current session.
#[derive(Clone, Copy, Debug)]
pub struct Sample {
    /// Increments with every published sample; gaps mean the application
    /// skipped samples.
    pub revision: u32,
    pub acceleration_m_s2: Option<[f32; 3]>,
    pub angular_velocity_deg_s: Option<[f32; 3]>,
    pub magnetic_field_ut: Option<[f32; 3]>,
    pub status: Status,
    pub orientation: Orientation,
    pub mag_status: MagStatus,
    /// Magnitude of the (calibrated) magnetic field.
    pub mag_field_strength_ut: f32,
    /// Magnetometer calibration progress, 100 once a model is in use.
    pub mag_calibration_percent: u8,
}

/// Physical measurements of one sample before fusion.
#[derive(Clone, Copy, Debug, Default)]
struct Measurements {
    acceleration_m_s2: Option<[f32; 3]>,
    angular_velocity_deg_s: Option<[f32; 3]>,
    magnetic_field_ut: Option<[f32; 3]>,
}
