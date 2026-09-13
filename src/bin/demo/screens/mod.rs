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

pub(crate) trait Screen {
    /// The screen is about to become visible.
    fn enter(&mut self) {}

    /// Another screen is about to take over.
    fn leave(&mut self) {}

    /// Called on every loop iteration, visible or not.
    fn update(&mut self, _now: Instant) {}

    /// A touch inside the content area while this screen is visible, in
    /// content coordinates.
    fn handle_touch(&mut self, _event: TouchEvent) {}

    /// Called on every loop iteration while visible. Redraw when something
    /// changed; do nothing otherwise. `canvas` is the size of `surface`.
    fn present(&mut self, canvas: &mut Canvas, surface: &mut Surface<'_>);
}
