//! Support code that is not about a particular piece of hardware.
//!
//! - [`logging`]: the `log` backend, with an on-device history.
//! - [`memory`]: PSRAM setup, allocation helpers and a usage report.

pub mod logging;
pub mod memory;
