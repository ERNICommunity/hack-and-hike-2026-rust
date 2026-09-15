//! Hack & Hike firmware library for the M5Stack CoreS3 Lite.
//!
//! An application is a binary in `src/bin/`. It calls [`Board::init`] once.
//! From the returned [`Board`], it keeps the capability handles it needs.
//! Then it runs its own loop:
//!
//! ```ignore
//! #[esp_rtos::main]
//! async fn main(_spawner: Spawner) -> ! {
//!     let Board { mut display, mut touch, .. } = Board::init();
//!     loop {
//!         // read input, update state, draw what changed
//!         Timer::after(Duration::from_millis(10)).await;
//!     }
//! }
//! ```
//!
//! # Where to look
//!
//! - [`capabilities`]: one module for each part of the hardware. Each module
//!   has a small handle type: [`Display`](capabilities::display::Display),
//!   [`Touch`](capabilities::touch::Touch), [`Imu`](capabilities::imu::Imu),
//!   [`Microphone`](capabilities::audio::Microphone),
//!   [`Speaker`](capabilities::audio::Speaker),
//!   [`Network`](capabilities::network::Network),
//!   [`Camera`](capabilities::camera::Camera),
//!   [`Light`](capabilities::light::Light),
//!   [`Proximity`](capabilities::proximity::Proximity) and
//!   [`Backlight`](capabilities::backlight::Backlight).
//! - [`ui`]: a [`Canvas`](ui::Canvas) to draw on, the colour palette, text
//!   helpers and a slider.
//! - [`synth`]: sine waves for the speaker.
//! - [`logging`]: the log history and a memory usage report.
//! - [`psram`]: long-lived buffers in PSRAM, the external RAM chip.
//!
//! All hardware details (pins, power rails, the second CPU core) stay inside
//! this library. Applications never need `esp_hal`.

#![no_std]
#![warn(missing_docs)]
#![warn(clippy::missing_docs_in_private_items)]

extern crate alloc;

// The panic handler. It prints the panic message and a backtrace on the USB
// serial port. Nothing in the crate calls it; `as _` only links it in.
use esp_backtrace as _;

mod board;
pub mod capabilities;
pub mod logging;
pub mod synth;
pub mod ui;

// Both live in `src/board/`. The rest of that module stays private.
pub use board::{Board, psram};
