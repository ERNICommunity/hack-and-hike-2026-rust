//! Physical facts about the M5Stack CoreS3 Lite board.
//!
//! This module owns what is specific to the PCB rather than to a chip: the
//! shared I2C bus, the power rails of the AXP2101 power chip and the reset
//! lines behind the AW9523 IO expander. Register setup of a device stays with
//! the capability that uses it: LCD controller setup in `display`, codec
//! setup in `audio`, sensor setup in `imu`.
//!
//! Nothing here is visible to applications.

pub(crate) mod i2c;
pub(crate) mod io_expander;
pub(crate) mod power;
pub(crate) mod registers;

/// Width of the LCD and of the touch panel's coordinate space.
pub(crate) const DISPLAY_WIDTH: usize = 320;
/// Height of the LCD and of the touch panel's coordinate space.
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
