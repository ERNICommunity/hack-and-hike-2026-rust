//! Hardware-independent logic of the Hack & Hike firmware.
//!
//! Everything here is plain `no_std` Rust with no ESP32 dependency, so it
//! compiles for your computer too and has ordinary tests:
//!
//! ```text
//! ./scripts/test.sh
//! ```
//!
//! The firmware crate wraps these types with the hardware drivers:
//!
//! | Module | Contents | Used by |
//! | --- | --- | --- |
//! | [`audio`] | the speaker's ring buffer, an ADPCM decoder | audio capability, demo |
//! | [`imu`] | sensor fusion, magnetometer compensation and calibration | IMU capability |
//! | [`light`] | data decoding, lux formula and proximity scale of the light sensor | light and proximity capabilities |
//! | [`lines`] | a fixed-size history of text lines | log history |
//! | [`network`] | wire protocol, typed messages, peer table | network capability |
//! | [`touch`] | decoding of the touch controller's report | touch capability |
//!
//! A good place for new logic that deserves tests: if it does not need the
//! hardware, put it here, test it on your computer, and call it from the
//! firmware.

#![no_std]
#![warn(missing_docs)]
#![warn(clippy::missing_docs_in_private_items)]

pub mod audio;
pub mod imu;
pub mod light;
pub mod lines;
pub mod network;
pub mod touch;
