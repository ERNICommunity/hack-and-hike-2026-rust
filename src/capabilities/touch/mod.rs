//! CPU1 touch-input capability.
//!
//! The facade exposes semantic [`TouchPoint`] / [`TouchEdge`] values and a
//! move-only [`Touch`] reader to applications. FT6336 polling and Embassy
//! synchronization stay private to this capability.

mod channels;
mod task;

pub use channels::Touch;
pub(crate) use channels::{Endpoints, Runtime, init_endpoints};
pub(crate) use task::capture_task;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TouchPoint {
    pub x: u16,
    pub y: u16,
}

#[derive(Clone, Copy, Debug)]
pub enum TouchEdge {
    Pressed(TouchPoint),
    Released(TouchPoint),
}
