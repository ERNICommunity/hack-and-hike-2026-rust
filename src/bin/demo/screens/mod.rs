//! The demo's screens.
//!
//! Each screen owns the capability handles it needs, keeps its own state and
//! draws itself. The shell in `main.rs` only routes touches and decides which
//! screen is visible. To add a screen: write a type that implements
//! [`Screen`], add it to `Screens` in `main.rs` and give it a `ViewId`.

pub(crate) mod camera;
pub(crate) mod imu;
pub(crate) mod log;
pub(crate) mod microphone;
pub(crate) mod network;
pub(crate) mod settings;
pub(crate) mod speaker;

use embassy_time::Instant;
use hack_and_hike::{
    capabilities::{display::Surface, touch::TouchEvent},
    ui::Canvas,
};

/// What every demo screen does. All methods but `present` have an empty
/// default, so a screen implements only what it needs.
///
/// The shell calls them in this order on every loop iteration:
/// `handle_touch` (visible screen, for each touch), `update` (every screen),
/// `present` (visible screen). `enter` and `leave` bracket the time a screen
/// is visible.
pub(crate) trait Screen {
    /// The screen is about to become visible.
    fn enter(&mut self) {}

    /// Another screen is about to take over.
    fn leave(&mut self) {}

    /// Called on every loop iteration, visible or not.
    fn update(&mut self, _now: Instant) {}

    /// Whether the loop may pause between iterations while this screen is
    /// visible. The camera screen says no: the sensor streams into a buffer
    /// of a few milliseconds, and a pause would let it overflow.
    fn may_idle(&self) -> bool {
        true
    }

    /// A touch inside the content area while this screen is visible, in
    /// content coordinates.
    fn handle_touch(&mut self, _event: TouchEvent) {}

    /// Called on every loop iteration while visible. Redraw when something
    /// changed; do nothing otherwise. `canvas` is the size of `surface`.
    fn present(&mut self, canvas: &mut Canvas, surface: &mut Surface<'_>);
}
