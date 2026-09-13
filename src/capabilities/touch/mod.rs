//! Touch panel capability.
//!
//! The FT6336 controller is polled on CPU1. The application receives
//! [`TouchEvent`]s in display coordinates through the [`Touch`] handle, in
//! the order they happened: a press, any number of moves, a release.

mod runtime;

use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, channel::Channel};
use embedded_graphics::prelude::Point;
use log::debug;

pub(crate) use runtime::spawn;

/// Events queue up until the application reads them. Moves are dropped
/// first when the queue fills; presses and releases are kept as long as
/// possible.
const QUEUE_LENGTH: usize = 32;
const MOVE_HEADROOM: usize = 4;

/// One finger touching down, moving or lifting off, with its position in
/// display coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TouchEvent {
    Pressed(Point),
    Moved(Point),
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

struct Service {
    events: Channel<CriticalSectionRawMutex, TouchEvent, QUEUE_LENGTH>,
}

static SERVICE: Service = Service {
    events: Channel::new(),
};

/// Application handle for the touch panel.
pub struct Touch {
    service: &'static Service,
}

impl Touch {
    /// The next queued touch event, if any.
    pub fn next_event(&mut self) -> Option<TouchEvent> {
        self.service.events.try_receive().ok()
    }
}

/// CPU1 side of the queue.
#[derive(Clone, Copy)]
pub(crate) struct Runtime {
    service: &'static Service,
}

impl Runtime {
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

pub(crate) struct Endpoints {
    pub(crate) handle: Touch,
    pub(crate) runtime: Runtime,
}

pub(crate) fn endpoints() -> Endpoints {
    Endpoints {
        handle: Touch { service: &SERVICE },
        runtime: Runtime { service: &SERVICE },
    }
}
