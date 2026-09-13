//! Physical facts about the M5Stack CoreS3 Lite board.
//!
//! This module owns pins, power rails, reset lines and the shared I2C bus.
//! Device register setup stays with the owning capability: LCD controller
//! setup in `display`, codec setup in `audio`, sensor setup in `imu`.

pub(crate) mod i2c;
pub(crate) mod io_expander;
pub(crate) mod power;

/// Native LCD/touch coordinate space of the CoreS3 Lite panel.
pub(crate) const DISPLAY_WIDTH: usize = 320;
pub(crate) const DISPLAY_HEIGHT: usize = 240;

/// Physical mounting orientation used by both LCD setup and touch coordinates.
///
/// Keeping this board fact shared prevents the rendered content and FT6336 touch
/// positions from drifting into different coordinate systems.
pub(crate) const DISPLAY_ROTATED_180: bool = true;

/// Convert a point from the touch controller's native panel coordinates into the
/// logical display coordinates consumed by presentation code.
pub(crate) const fn logical_display_point(x: u16, y: u16) -> (u16, u16) {
    if DISPLAY_ROTATED_180 {
        (DISPLAY_WIDTH as u16 - 1 - x, DISPLAY_HEIGHT as u16 - 1 - y)
    } else {
        (x, y)
    }
}
