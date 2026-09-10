//! Runtime hardware ownership expressed through concrete CPU resource bundles.
//!
//! Bootstrap moves each raw peripheral exactly once to the CPU-side bundle that
//! owns it: CPU0 keeps display/camera I/O; CPU1 receives runtime I2C, audio
//! acquisition, and radio. Touch and IMU intentionally do not have independent
//! raw-I2C handles because both consume the shared CPU1-local `SystemI2cBus`.

use crate::{
    platform::i2c as system_i2c,
    services::{audio, camera, display, network},
};

/// Raw peripherals that stay on CPU0.
pub(super) struct Cpu0Resources {
    pub(super) display: display::Resources,
    pub(super) camera: camera::Resources,
}

/// Raw peripherals moved into the CPU1 service executor.
pub(super) struct Cpu1Resources {
    pub(super) system_i2c: system_i2c::Resources<'static>,
    pub(super) audio: audio::Resources,
    pub(super) network: network::Resources,
}
