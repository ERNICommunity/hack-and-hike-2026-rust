//! Hack & Hike firmware library for the M5Stack CoreS3 Lite.
//!
//! An application is a binary in `src/bin/`. It calls [`Board::init`], takes
//! the capability handles it needs from the returned [`Board`] and runs its
//! own loop:
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
//! - [`capabilities`]: one module per piece of hardware, each with a small
//!   handle type: [`Display`](capabilities::display::Display),
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
//! - [`psram`]: long-lived buffers in the external PSRAM.
//!
//! Everything hardware-specific (pins, power rails, the second CPU core)
//! stays inside this library; applications never need `esp_hal`.

#![no_std]
#![warn(missing_docs)]
#![warn(clippy::missing_docs_in_private_items)]

extern crate alloc;

// The panic handler: prints the message and a backtrace on the USB serial
// port. Linked in for its side effect only.
use esp_backtrace as _;

mod board;
pub mod capabilities;
pub mod logging;
pub mod synth;
pub mod ui;

// Both live in `src/board/`; the rest of that module stays private.
pub use board::{Board, psram};
