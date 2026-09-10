//! CPU1 touch-input capability.
//!
//! The facade exposes semantic touch points/edges and the CPU0 input endpoint.
//! FT6336 polling and Embassy synchronization stay private to this capability.

mod channels;
mod task;

pub use channels::Input;
pub use task::capture_task;
pub(crate) use channels::{Endpoints, Runtime, init_endpoints};

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
