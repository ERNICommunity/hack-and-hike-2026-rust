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
//! Service-specific raw hardware requirements live with the owning service as
//! `screen::Resources`, `audio::Resources`, and `system_i2c::Resources`. The IMU
//! has no separate raw peripheral bundle: it is a CPU1 service using the shared
//! CPU1-local `system_i2c::SystemI2cBus`.

use crate::{audio, cross_core, screen, system_i2c};

/// Complete runtime split between the two architectural sides.
///
/// ```text
/// RuntimeResources
/// ├── Cpu0Resources
/// │   ├── screen::Resources
/// │   └── Cpu0AppEndpoint
/// └── Cpu1Resources
///     ├── system_i2c::Resources  (touch + IMU runtime bus)
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
/// The IMU deliberately does not appear as a raw resource field because its
/// hardware transport is the shared runtime system-I2C bus. Future ESP-NOW raw
/// resources should be represented by a resource type defined by that service.
pub struct Cpu1Resources {
    pub system_i2c: system_i2c::Resources,
    pub audio: audio::Resources,
    pub services: cross_core::Cpu1ServiceEndpoint,
}
