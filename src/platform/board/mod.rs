//! Board-level PCB policy for the M5Stack CoreS3 Lite.
//!
//! This module owns physical board facts, power rails, resets, enables, and
//! other wiring policy. Device register configuration remains with the owning
//! service: LCD controller setup in `display`, ES7210 setup in `audio`, and IMU
//! device setup in `imu`.

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
        (
            DISPLAY_WIDTH as u16 - 1 - x,
            DISPLAY_HEIGHT as u16 - 1 - y,
        )
    } else {
        (x, y)
    }
}

pub(crate) mod io_expander;
pub(crate) mod power;
