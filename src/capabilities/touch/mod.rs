//! Touch panel capability.
//!
//! Applications see [`TouchPoint`] positions and [`TouchEdge`] press/release
//! events through the [`Touch`] handle. FT6336 polling runs on CPU1.

mod channels;
mod task;

pub use channels::Touch;
pub(crate) use channels::{Endpoints, Runtime, endpoints};
pub(crate) use task::capture_task;

/// A finger position in display coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TouchPoint {
    pub x: u16,
    pub y: u16,
}

/// A finger touching down or lifting off, with the position where it happened.
#[derive(Clone, Copy, Debug)]
pub enum TouchEdge {
    Pressed(TouchPoint),
    Released(TouchPoint),
}
