//! Physical facts about the M5Stack CoreS3 Lite board.
//!
//! This module owns pins, power rails, reset lines and the shared I2C bus.
//! Device register setup stays with the owning capability: LCD controller
//! setup in `display`, codec setup in `audio`, sensor setup in `imu`.

pub(crate) mod i2c;
pub(crate) mod io_expander;
pub(crate) mod power;
pub(crate) mod registers;

/// Native LCD/touch coordinate space of the CoreS3 Lite panel.
pub(crate) const DISPLAY_WIDTH: usize = 320;
pub(crate) const DISPLAY_HEIGHT: usize = 240;

/// The panel is mounted upside down relative to its controller's native
/// orientation. Both the LCD setup and the touch coordinates follow this, so
/// what is drawn and what is touched share one coordinate system.
pub(crate) const DISPLAY_ROTATED_180: bool = true;

/// Convert a point from the touch controller's native panel coordinates into
/// display coordinates.
pub(crate) const fn logical_display_point(x: u16, y: u16) -> (u16, u16) {
    if DISPLAY_ROTATED_180 {
        hack_and_hike_core::touch::rotate_180(x, y, DISPLAY_WIDTH as u16, DISPLAY_HEIGHT as u16)
    } else {
        (x, y)
    }
}
