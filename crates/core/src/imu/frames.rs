//! Coordinate frames used by the IMU.
//!
//! ```text
//! body frame    the BMI270's own axes as mounted on the board
//! screen frame  x: out of the top edge of the device (camera direction)
//!               y: across the screen
//!               z: into the screen (points down when the device lies flat)
//! ```
//!
//! Fusion works in the screen frame so that "gravity" and "north" mean
//! something to the application; sensors deliver body-frame values.

/// Express a body-frame vector in the screen frame.
pub fn screen_from_body(v: [f32; 3]) -> [f32; 3] {
    [v[2], -v[0], -v[1]]
}

/// Express a screen-frame vector in the body frame.
pub fn body_from_screen(v: [f32; 3]) -> [f32; 3] {
    [-v[1], -v[2], v[0]]
}

/// Express a BMM150 magnetometer reading in the body frame. The magnetometer
/// shares the x axis with the BMI270 and has y and z pointing the other way.
pub fn body_from_magnetometer(v: [f32; 3]) -> [f32; 3] {
    [v[0], -v[1], -v[2]]
}
