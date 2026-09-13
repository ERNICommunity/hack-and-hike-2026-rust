//! CPU1-to-CPU0 touch event and point synchronization.

use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex, channel::Channel, signal::Signal,
};
use log::debug;

use super::{TouchEdge, TouchPoint};

/// Press and release events queue up until the application reads them.
const EDGE_QUEUE_LENGTH: usize = 8;

struct Service {
    edges: Channel<CriticalSectionRawMutex, TouchEdge, EDGE_QUEUE_LENGTH>,
    latest_point: Signal<CriticalSectionRawMutex, TouchPoint>,
}

static SERVICE: Service = Service {
    edges: Channel::new(),
    latest_point: Signal::new(),
};

/// Application handle for the touch panel.
///
/// Press and release events are queued; the finger position is replace-latest.
pub struct Touch {
    service: &'static Service,
}

impl Touch {
    /// The next press or release event, if any is queued.
    pub fn next_edge(&mut self) -> Option<TouchEdge> {
        self.service.edges.try_receive().ok()
    }

    /// The newest finger position since the last call, if it changed.
    pub fn take_latest_point(&mut self) -> Option<TouchPoint> {
        self.service.latest_point.try_take()
    }
}

#[derive(Clone, Copy)]
pub(crate) struct Runtime {
    service: &'static Service,
}

impl Runtime {
    pub(super) fn publish_edge(self, edge: TouchEdge) {
        if self.service.edges.try_send(edge).is_err() {
            debug!("Touch event dropped: the application is not reading events");
        }
    }

    pub(super) fn publish_point(self, point: TouchPoint) {
        self.service.latest_point.signal(point);
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
