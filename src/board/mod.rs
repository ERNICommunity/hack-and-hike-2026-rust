//! Board-level PCB policy for the M5Stack CoreS3 Lite.
//!
//! This module owns physical board facts, power rails, resets, enables, and
//! other wiring policy. Device register configuration remains with the owning
//! service: LCD controller setup in `display`, ES7210 setup in `audio`, and IMU
//! device setup in `imu`.

/// Native LCD/touch coordinate space of the CoreS3 Lite panel.
pub const DISPLAY_WIDTH: usize = 320;
pub const DISPLAY_HEIGHT: usize = 240;

pub mod io_expander;
pub mod power;
