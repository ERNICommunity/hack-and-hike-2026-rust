//! Hardware capabilities exposed to applications.
//!
//! Every capability follows the same shape:
//!
//! - A **handle** (`Display`, `Touch`, `Imu`, ...) is what the application
//!   owns and calls. Handles are move-only values returned by
//!   [`crate::Board::init`].
//! - A **runtime** is the private CPU1 side that talks to the hardware and
//!   publishes data for the handle. Applications never see it.
//!
//! Hardware details such as I2C registers, DMA channels and pin numbers stay
//! inside the capability.

pub mod audio;
pub mod backlight;
pub mod camera;
pub mod display;
pub mod imu;
pub mod light;
pub mod network;
pub mod touch;
