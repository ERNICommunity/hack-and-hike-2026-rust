//! Hardware/runtime capabilities exposed to applications.
//!
//! A capability owns hardware/runtime implementation and exposes a semantic data API to applications.

pub(crate) mod audio;
pub mod camera;
pub mod display;
pub mod imu;
pub mod mic;
pub mod network;
pub mod speaker;
pub mod touch;
