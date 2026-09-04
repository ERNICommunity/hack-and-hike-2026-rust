//! Runtime ownership expressed through concrete resource bundles.
//!
//! These types describe the architecture without trying to prove physical CPU
//! affinity. Bootstrap code is responsible for moving each bundle to the
//! intended core.
//!
//! - CPU0 owns application/model coordination, Slint presentation, display I/O,
//!   direct display overlays, and the application side of semantic cross-core
//!   communication.
//! - CPU1 owns non-display peripheral services, timing-sensitive acquisition or
//!   communication, runtime system I2C, and the service side of semantic
//!   cross-core communication.
//!
//! Service-specific hardware requirements live with the owning service as
//! `screen::Resources`, `audio::Resources`, and `system_i2c::Resources`.

use crate::{audio, cross_core, screen, system_i2c};

/// Complete runtime split between the two architectural sides.
///
/// ```text
/// RuntimeResources
/// ├── Cpu0Resources
/// │   ├── screen::Resources
/// │   └── Cpu0AppEndpoint
/// └── Cpu1Resources
///     ├── system_i2c::Resources
///     ├── audio::Resources
///     └── Cpu1ServiceEndpoint
/// ```
pub struct RuntimeResources {
    pub cpu0: Cpu0Resources,
    pub cpu1: Cpu1Resources,
}

/// Resources belonging to CPU0's application/presentation side.
pub struct Cpu0Resources {
    pub display: screen::Resources,
    pub app: cross_core::Cpu0AppEndpoint,
}

/// Resources belonging to CPU1's non-display service side.
///
/// Future IMU and ESP-NOW resources belong here and should be represented by
/// resource types defined by those owning service modules.
pub struct Cpu1Resources {
    pub system_i2c: system_i2c::Resources,
    pub audio: audio::Resources,
    pub services: cross_core::Cpu1ServiceEndpoint,
}
