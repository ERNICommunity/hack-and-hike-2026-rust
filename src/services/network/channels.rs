//! CPU1-to-CPU0 network snapshot transport.

use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use static_cell::StaticCell;

use super::Snapshot;

type SnapshotSignal = Signal<CriticalSectionRawMutex, Snapshot>;

struct Service {
    latest: SnapshotSignal,
}

impl Service {
    const fn new() -> Self {
        Self {
            latest: Signal::new(),
        }
    }
}

static SERVICE: StaticCell<Service> = StaticCell::new();

#[derive(Clone, Copy)]
pub(crate) struct Runtime {
    service: &'static Service,
}

pub struct Input {
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
    /// Take the newest network snapshot, if CPU1 published one since the
    /// previous take. Multiple CPU1 updates collapse to one latest value.
    pub fn take_latest(&mut self) -> Option<Snapshot> {
        self.service.latest.try_take()
    }
}

impl Runtime {
    pub(super) fn publish(self, snapshot: Snapshot) {
        self.service.latest.signal(snapshot);
    }
}
