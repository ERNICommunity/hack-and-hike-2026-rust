//! The touch panel.
//!
//! The FT6336 controller is polled every 5 ms on CPU1. The application
//! receives [`TouchEvent`]s through the [`Touch`] handle, in the order they
//! happened: a press, any number of moves, a release.
//!
//! Positions are in display coordinates, so a point can be compared directly
//! with what was drawn there. Only the first finger is reported.
//!
//! ```ignore
//! while let Some(event) = touch.next_event() {
//!     if let TouchEvent::Pressed(point) = event {
//!         log::info!("tap at {},{}", point.x, point.y);
//!     }
//! }
//! ```

mod runtime;

use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, channel::Channel};
use embedded_graphics::prelude::Point;
use log::debug;

pub(crate) use runtime::spawn;

/// Events queue up until the application reads them.
const QUEUE_LENGTH: usize = 32;
/// Queue slots kept free for presses and releases: when fewer are left, new
/// moves are dropped. An application that reads slowly then sees jumps in a
/// drag, but never a press without its release.
const MOVE_HEADROOM: usize = 4;

/// One finger touching down, moving or lifting off, with its position in
/// display coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TouchEvent {
    /// A finger touched the panel here.
    Pressed(Point),
    /// The finger moved here while still touching.
    Moved(Point),
    /// The finger lifted off; the point is where it was last seen.
    Released(Point),
}

impl TouchEvent {
    /// The position of the finger for this event.
    pub const fn point(self) -> Point {
        match self {
            Self::Pressed(point) | Self::Moved(point) | Self::Released(point) => point,
        }
    }
}

/// The queue shared by the handle (CPU0) and the touch polling task (CPU1).
struct Service {
    /// Touch events waiting for the application, oldest first.
    events: Channel<CriticalSectionRawMutex, TouchEvent, QUEUE_LENGTH>,
}

/// The one touch queue. A plain `static` works across cores because the
/// channel synchronizes itself.
static SERVICE: Service = Service {
    events: Channel::new(),
};

/// Application handle for the touch panel; see the [module docs](self).
pub struct Touch {
    /// Points at the queue shared with the CPU1 touch polling task.
    service: &'static Service,
}

impl Touch {
    /// The oldest unread touch event, or `None` when there is none. Never
    /// waits.
    ///
    /// Read all waiting events on every loop iteration, with `while let`.
    pub fn next_event(&mut self) -> Option<TouchEvent> {
        self.service.events.try_receive().ok()
    }
}

/// CPU1 side of the queue.
#[derive(Clone, Copy)]
pub(crate) struct Runtime {
    /// Points at the queue shared with the application's handle on CPU0.
    service: &'static Service,
}

impl Runtime {
    /// Queue `event` for the application, dropping moves when the queue is
    /// nearly full.
    fn publish(self, event: TouchEvent) {
        let events = &self.service.events;
        if matches!(event, TouchEvent::Moved(_)) && events.free_capacity() < MOVE_HEADROOM {
            return;
        }
        if events.try_send(event).is_err() {
            debug!("Touch event dropped: the application is not reading events");
        }
    }
}

/// The two ends of the touch queue, created once by the board.
pub(crate) struct Endpoints {
    /// For the application.
    pub(crate) handle: Touch,
    /// For the CPU1 polling task.
    pub(crate) runtime: Runtime,
}

/// Both ends of the touch queue.
pub(crate) fn endpoints() -> Endpoints {
    Endpoints {
        handle: Touch { service: &SERVICE },
        runtime: Runtime { service: &SERVICE },
    }
}
