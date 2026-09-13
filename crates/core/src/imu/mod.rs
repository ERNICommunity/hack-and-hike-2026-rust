//! Orientation math: sensor fusion, gyro-bias learning and magnetometer
//! calibration.

pub mod bmm150;
pub mod frames;
pub mod fusion;
pub mod vec3;

/// The device's orientation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Orientation {
    /// Euler angles are for display only. They have singularities near the
    /// poles and must not be used to reconstruct a 3-D pose.
    pub roll_deg: f32,
    pub pitch_deg: f32,
    /// Magnetic heading; no declination correction is applied.
    pub yaw_deg: f32,
    /// World gravity (down) expressed in the screen frame.
    pub gravity_screen: [f32; 3],
    /// Magnetic north expressed in the screen frame, perpendicular to
    /// gravity. Well-defined even where the Euler angles are not.
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
