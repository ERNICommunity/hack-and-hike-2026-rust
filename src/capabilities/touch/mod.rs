//! CPU1 touch-input capability.
//!
//! The facade exposes semantic [`TouchPoint`] / [`TouchEdge`] values and a
//! move-only [`Touch`] reader to applications. FT6336 polling and Embassy
//! synchronization stay private to this capability.

mod channels;
mod task;

pub(crate) use channels::Touch;
// Bootstrap still names its aggregate field `input`; keep this internal alias so
// composition terminology does not leak into the application-facing API.
pub(crate) type Input = Touch;
pub(crate) use channels::{Endpoints, Runtime, init_endpoints};
pub(crate) use task::capture_task;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TouchPoint {
    pub(crate) x: u16,
    #[allow(dead_code, reason = "coordinate is part of the application-facing touch capability contract")]
    pub(crate) y: u16,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum TouchEdge {
    Pressed(TouchPoint),
    Released(
        #[allow(dead_code, reason = "release position is part of the application-facing touch capability contract")]
        TouchPoint,
    ),
}
