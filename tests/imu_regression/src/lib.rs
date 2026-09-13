//! Host tests for the hardware-independent IMU math.
//!
//! The production modules are compiled unchanged from `src/`; they depend only
//! on each other and on `libm`.
#![allow(
    dead_code,
    reason = "the modules' API is used by the firmware, not by this harness; the next phase moves them into a library crate with tests next to the code"
)]

/// Stand-in for `capabilities::imu::Orientation`, which fusion produces.
#[derive(Clone, Copy, Debug, Default)]
struct Orientation {
    roll_deg: f32,
    pitch_deg: f32,
    yaw_deg: f32,
    gravity_screen: [f32; 3],
    north_screen: [f32; 3],
}

#[path = "../../../src/bin/demo/views/imu_worldview/view/attitude.rs"]
mod attitude;
#[path = "../../../src/capabilities/imu/frames.rs"]
mod frames;
#[path = "../../../src/capabilities/imu/fusion.rs"]
mod fusion;
#[path = "../../../src/capabilities/imu/vec3.rs"]
mod vec3;

#[cfg(test)]
mod attitude_tests;
#[cfg(test)]
mod fusion_tests;
