//! The touch panel.
//!
//! A CPU1 task reads the FT6336 touch controller every 5 ms. The application
//! gets [`TouchEvent`]s through the [`Touch`] handle, in the order they
//! happened: a press, any number of moves, then a release.
//!
//! Positions are in display coordinates, so you can compare a point directly
//! with what you drew there. Only the first finger is reported.
//!
//! ```ignore
//! while let Some(event) = touch.next_event() {
//!     if let TouchEvent::Pressed(point) = event {
//!         log::info!("tap at {},{}", point.x, point.y);
//!     }
//! }
//! ```
//!
//! # When the application reads too slowly
//!
//! Up to 32 events wait in a queue. When the queue is nearly full, new moves
//! are dropped first, so the application sees jumps in a drag. A press is
//! dropped when its release would not fit behind it. So a press always
//! comes with its release. When a press is dropped, its moves and its
//! release are dropped too, and the press is tried again while the finger
//! stays down.

mod runtime;

use core::sync::atomic::{AtomicBool, Ordering};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, channel::Channel};
use embedded_graphics::prelude::Point;

use log::warn;

pub(crate) use runtime::spawn;

/// Number of events that can wait in the queue until the application reads
/// them.
const QUEUE_LENGTH: usize = 32;
/// Free queue slots that a move needs. When fewer slots are free, new moves
/// are dropped, and the last slots stay free for presses and releases. An
/// application that reads slowly then sees jumps in a drag, but never a
/// press without its release.
const MOVE_HEADROOM: usize = 4;

/// A finger touches the panel, moves or lifts off. Each event has the
/// position in display coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TouchEvent {
    /// A finger touched the panel here.
    Pressed(Point),
    /// The finger moved to this new position while still touching.
    Moved(Point),
    /// The finger lifted off. The point is the last position where the
    /// finger was seen.
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
    /// A press or release was dropped and no event was queued since. The
    /// warning is logged only when this turns true, so an application that
    /// does not read touch at all does not fill the log.
    dropping: AtomicBool,
}

/// The one touch queue. A plain `static` is safe to use from both cores,
/// because the channel protects its items with a critical section.
static SERVICE: Service = Service {
    events: Channel::new(),
    dropping: AtomicBool::new(false),
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
    /// Read all waiting events on every loop iteration, with `while let`, so
    /// the queue does not fill up.
    pub fn next_event(&mut self) -> Option<TouchEvent> {
        self.service.events.try_receive().ok()
    }
}

/// CPU1 side of the queue, used by the touch polling task.
#[derive(Clone, Copy)]
pub(crate) struct Runtime {
    /// Points at the queue shared with the application's handle on CPU0.
    service: &'static Service,
}

impl Runtime {
    /// Queue `event` for the application. Returns whether the event was
    /// queued.
    ///
    /// Each kind of event needs a number of free slots:
    ///
    /// - A move needs [`MOVE_HEADROOM`] free slots. So moves are dropped
    ///   first when the queue is nearly full.
    /// - A press needs two free slots: one for itself and one for its
    ///   release.
    /// - A release needs one free slot. After a press, that slot is always
    ///   free, because only moves come between them.
    ///
    /// The first dropped press or release logs a warning. The next warning
    /// comes only after an event was queued again. Dropped moves are not
    /// logged: they only make a drag jump.
    fn publish(self, event: TouchEvent) -> bool {
        let events = &self.service.events;
        let needed = match event {
            TouchEvent::Moved(_) => MOVE_HEADROOM,
            TouchEvent::Pressed(_) => 2,
            TouchEvent::Released(_) => 1,
        };
        let dropped = events.free_capacity() < needed || events.try_send(event).is_err();
        if !dropped {
            self.service.dropping.store(false, Ordering::Relaxed);
        } else if !matches!(event, TouchEvent::Moved(_))
            && !self.service.dropping.swap(true, Ordering::Relaxed)
        {
            warn!(
                "Touch events dropped: the queue is full. Read them with `touch.next_event()` on every loop iteration."
            );
        }
        !dropped
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
