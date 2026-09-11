//! CPU1-to-CPU0 touch event and point synchronization.

use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex, channel::Channel, signal::Signal,
};
use static_cell::StaticCell;

use super::{TouchEdge, TouchPoint};

type EdgeChannel = Channel<CriticalSectionRawMutex, TouchEdge, 8>;
type PointSignal = Signal<CriticalSectionRawMutex, TouchPoint>;

// These outputs cross from CPU1 acquisition to CPU0 presentation, therefore
// they use CriticalSectionRawMutex. The static storage remains an Embassy
// implementation detail; bootstrap hands each side an endpoint referencing the
// concrete service instance.
struct Service {
    edges: EdgeChannel,
    latest_point: PointSignal,
}

impl Service {
    const fn new() -> Self {
        Self {
            edges: Channel::new(),
            latest_point: Signal::new(),
        }
    }
}

static SERVICE: StaticCell<Service> = StaticCell::new();

#[derive(Clone, Copy)]
pub(crate) struct Runtime {
    service: &'static Service,
}

pub(crate) struct Input {
    service: &'static Service,
}

pub(crate) struct Endpoints {
    pub(crate) runtime: Runtime,
    pub(crate) input: Input,
}

pub(crate) fn init_endpoints() -> Endpoints {
    let service: &'static Service = SERVICE.init(Service::new());
    Endpoints {
        runtime: Runtime { service },
        input: Input { service },
    }
}

impl Input {
    pub(crate) fn next_edge(&mut self) -> Option<TouchEdge> {
        self.service.edges.try_receive().ok()
    }

    pub(crate) fn take_latest_point(&mut self) -> Option<TouchPoint> {
        self.service.latest_point.try_take()
    }
}

impl Runtime {
    pub(super) fn try_publish_edge(self, edge: TouchEdge) -> bool {
        self.service.edges.try_send(edge).is_ok()
    }

    pub(super) fn publish_point(self, point: TouchPoint) {
        self.service.latest_point.signal(point);
    }
}
