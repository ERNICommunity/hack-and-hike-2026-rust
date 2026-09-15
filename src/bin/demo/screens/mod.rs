//! The demo's screens.
//!
//! Each screen owns the capability handles it needs, keeps its own state and
//! draws itself. The shell in `main.rs` only routes touches and decides which
//! screen is visible.
//!
//! To add a screen:
//!
//! 1. Write a type that implements [`Screen`].
//! 2. In `main.rs`, add a field for it to `Screens`, create it in `main`, and
//!    add it to `Screens::get_mut`.
//! 3. In `navigation.rs`, add a variant to `ViewId`, add it to `ViewId::ALL`,
//!    and give it a 16 x 16 icon in `ViewId::icon`.

pub(crate) mod camera;
pub(crate) mod imu;
pub(crate) mod log;
pub(crate) mod microphone;
pub(crate) mod network;
pub(crate) mod proximity;
pub(crate) mod settings;
pub(crate) mod speaker;

use embassy_time::Instant;
use hack_and_hike::{
    capabilities::{display::Surface, touch::TouchEvent},
    ui::Canvas,
};

/// What every demo screen does. All methods except `present` have an empty
/// default, so a screen implements only what it needs.
///
/// The shell calls the methods in this order on every loop iteration:
///
/// 1. `handle_touch`: for the visible screen, once for each touch event.
/// 2. `update`: for every screen.
/// 3. `present`: for the visible screen.
/// 4. A short pause, unless `may_idle` of the visible screen returns `false`.
///
/// `enter` runs when a screen becomes visible, and `leave` runs when another
/// screen takes over.
pub(crate) trait Screen {
    /// The screen is about to become visible.
    fn enter(&mut self) {}

    /// Another screen is about to take over.
    fn leave(&mut self) {}

    /// Called on every loop iteration, visible or not.
    fn update(&mut self, _now: Instant) {}

    /// Whether the loop may pause between iterations while this screen is
    /// visible. The camera screen returns `false` when there is a camera. The
    /// sensor sends data all the time into a buffer that holds only a few
    /// milliseconds, and a pause would let it overflow.
    fn may_idle(&self) -> bool {
        true
    }

    /// A touch inside the content area while this screen is visible, in
    /// content coordinates.
    fn handle_touch(&mut self, _event: TouchEvent) {}

    /// Called on every loop iteration while the screen is visible. Redraw when
    /// something changed, and return at once otherwise. `canvas` has the size
    /// of `surface`.
    fn present(&mut self, canvas: &mut Canvas, surface: &mut Surface<'_>);
}
