//! Runtime hardware ownership expressed through concrete resource bundles.
//!
//! `RuntimeResources` describes only raw-peripheral ownership. CPU1→CPU0 reader
//! handles live separately in `service_inputs::Cpu0Inputs` because peripheral
//! ownership and access to static cross-core data contracts are different
//! architectural concerns.
//!
//! Bootstrap moves each raw peripheral exactly once to the service that owns it:
//! CPU0 owns display/camera I/O; CPU1 owns runtime I2C, audio acquisition, and
//! radio. Touch and IMU intentionally do not have independent raw-I2C handles
//! because both consume the shared CPU1-local `SystemI2cBus`.

use crate::{audio, camera, display, network, system_i2c};

/// Complete raw-hardware ownership split created during bootstrap.
pub struct RuntimeResources {
    pub cpu0: Cpu0Resources,
    pub cpu1: Cpu1Resources,
}

/// Raw peripherals that stay on CPU0.
pub struct Cpu0Resources {
    pub display: display::Resources,
    pub camera: camera::Resources,
}

/// Raw peripherals moved into the CPU1 service executor.
///
/// Destructuring this value in bootstrap makes the ownership transfer explicit:
/// the display/camera side cannot retain the Wi-Fi/I2S/runtime-I2C peripherals.
pub struct Cpu1Resources {
    pub system_i2c: system_i2c::Resources,
    pub audio: audio::Resources,
    pub network: network::Resources,
}
