//! Orientation math: sensor fusion, gyro-bias learning and magnetometer
//! calibration.
//!
//! The firmware's IMU capability feeds raw sensor readings in and publishes
//! the result. The pieces, in the order a sample passes through them:
//!
//! 1. [`fusion::GyroBias`] learns the gyroscope's zero offset while the board
//!    lies still and removes it.
//! 2. [`bmm150::compensate`] turns the magnetometer's raw frame into
//!    microtesla using the chip's factory trim values.
//! 3. [`bmm150::Calibration`] learns the distortion of the enclosure while
//!    the board is moved around, and removes it.
//! 4. [`fusion::Fusion`] combines gyroscope, accelerometer and magnetometer
//!    into an [`Orientation`].
//!
//! [`frames`] converts between the axes of the sensors and of the screen, and
//! [`vec3`] has the small vector helpers everything above uses.

pub mod bmm150;
pub mod frames;
pub mod fusion;
pub mod vec3;

/// The device's orientation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Orientation {
    /// Roll in degrees, in the sensor's body frame.
    ///
    /// Euler angles are for display only. They have singularities near the
    /// poles and must not be used to reconstruct a 3-D pose.
    pub roll_deg: f32,
    /// Pitch in degrees, in the sensor's body frame; see `roll_deg`.
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
