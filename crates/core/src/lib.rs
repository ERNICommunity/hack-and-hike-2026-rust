//! Hardware-independent logic of the Hack & Hike firmware.
//!
//! Everything here is plain `no_std` Rust with no ESP32 dependency, so it is
//! unit-tested on your computer. The firmware crate wraps these types with the
//! hardware drivers.

#![no_std]

pub mod imu;
pub mod network;
