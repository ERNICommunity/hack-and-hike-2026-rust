//! Hack & Hike firmware library for the M5Stack CoreS3 Lite.
//!
//! An application is a binary in `src/bin/`. It calls [`Board::init`], takes
//! the capability handles it needs from the returned [`Board`], and runs its
//! own loop. Everything hardware-specific lives in this library.

#![no_std]

extern crate alloc;

// Panic handler: prints a backtrace over the USB serial port.
use esp_backtrace as _;

mod board;
pub mod capabilities;
pub(crate) mod platform;
pub mod support;
pub mod ui;

pub use board::Board;
