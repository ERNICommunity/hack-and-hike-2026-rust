//! Board-level PCB policy for the M5Stack CoreS3 Lite.
//!
//! This module owns power rails, resets, enables, and other board wiring policy.
//! Device register configuration remains with the owning service:
//! LCD controller setup in `screen`, ES7210 setup in `audio`, and future IMU
//! device setup in `imu`.

pub mod io_expander;
pub mod power;
