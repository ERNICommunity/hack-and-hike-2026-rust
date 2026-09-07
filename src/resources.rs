//! Runtime hardware ownership expressed through concrete resource bundles.
//!
//! Bootstrap code moves each raw peripheral exactly once to the service that
//! owns it. Cross-core data exchange is deliberately not represented here:
//! touch, IMU, audio, and network each expose their own bounded service-specific
//! Signal/Channel contract.
//!
//! - CPU0 owns application/model coordination, presentation state, and display
//!   I/O.
//! - CPU1 owns non-display peripheral services, timing-sensitive acquisition or
//!   communication, and runtime system I2C.
//!
//! Service-specific raw hardware requirements live with the owning service as
//! `display::Resources`, `audio::Resources`, `network::Resources`, and
//! `system_i2c::Resources`. The IMU has no separate raw peripheral bundle: it is
//! a CPU1 service using the shared CPU1-local `system_i2c::SystemI2cBus`.

use crate::{audio, display, network, system_i2c};

/// Complete raw-hardware split between the two architectural sides.
///
/// ```text
/// RuntimeResources
/// ├── Cpu0Resources
/// │   └── display::Resources
/// └── Cpu1Resources
///     ├── system_i2c::Resources  (touch + IMU runtime bus)
///     ├── audio::Resources
///     └── network::Resources     (ESP-NOW radio)
/// ```
pub struct RuntimeResources {
    pub cpu0: Cpu0Resources,
    pub cpu1: Cpu1Resources,
}

/// Raw peripherals belonging to CPU0's display side.
pub struct Cpu0Resources {
    pub display: display::Resources,
}

/// Raw peripherals belonging to CPU1's non-display service side.
pub struct Cpu1Resources {
    pub system_i2c: system_i2c::Resources,
    pub audio: audio::Resources,
    pub network: network::Resources,
}
