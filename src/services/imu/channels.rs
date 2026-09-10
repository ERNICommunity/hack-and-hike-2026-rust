//! CPU1-to-CPU0 IMU snapshot transport.

use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use static_cell::StaticCell;

use super::{MagStatus, Orientation, Snapshot, Status};

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
    /// Take the newest orientation/status snapshot, if CPU1 published one since
    /// the previous take. Multiple CPU1 updates collapse to one latest value.
    pub fn take_latest(&mut self) -> Option<Snapshot> {
        self.service.latest.try_take()
    }
}

pub(super) fn publish(
    runtime: Runtime,
    revision: &mut u32,
    status: Status,
    orientation: Orientation,
    mag_status: MagStatus,
    mag_field_ut: f32,
    mag_calibration_percent: u8,
) {
    *revision = revision.wrapping_add(1);
    runtime.service.latest.signal(Snapshot {
        revision: *revision,
        status,
        orientation,
        mag_status,
        mag_field_ut,
        mag_calibration_percent,
    });
}
