//! The hardware capabilities that applications use.
//!
//! Every capability is built the same way:
//!
//! - A **handle** (`Display`, `Touch`, `Imu`, ...) is the value that the
//!   application owns and calls. [`crate::Board::init`] returns the handles.
//!   A handle can be moved, but not copied.
//! - A **runtime** is the private part on CPU1, the second CPU core. It talks
//!   to the hardware and publishes data for the handle. Applications never
//!   see it. Handles that use a runtime never wait for the hardware.
//!
//! The display and the camera have no runtime. They do their work on CPU0,
//! inside the calls of the application, and these calls can wait.
//!
//! Hardware details stay inside the library. Chip registers stay inside the
//! capability. Pin numbers and DMA channels are chosen in
//! [`Board::init`](crate::Board::init), which gives them to the capability.

pub mod audio;
pub mod backlight;
pub mod camera;
pub mod display;
pub mod imu;
pub mod light;
pub mod network;
pub mod proximity;
pub mod touch;
