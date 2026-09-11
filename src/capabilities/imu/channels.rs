//! CPU1-to-CPU0 IMU latest-value transport.

use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use static_cell::StaticCell;

use super::{MagStatus, Measurements, Orientation, Sample, Status};

type SampleSignal = Signal<CriticalSectionRawMutex, Sample>;

struct Service {
    latest: SampleSignal,
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

/// CPU0 semantic IMU reader. Hardware polling, sensor register formats and
/// cross-core synchronization remain private to the capability.
pub(crate) struct Imu {
    service: &'static Service,
}

pub(crate) struct Endpoints {
    pub(crate) runtime: Runtime,
    pub(crate) input: Imu,
}

pub(crate) fn init_endpoints() -> Endpoints {
    let service: &'static Service = SERVICE.init(Service::new());
    Endpoints {
        runtime: Runtime { service },
        input: Imu { service },
    }
}

impl Imu {
    /// Take the newest coherent semantic IMU sample, if CPU1 published one since
    /// the previous take. Multiple CPU1 updates collapse to one latest value.
    pub(crate) fn latest(&mut self) -> Option<Sample> {
        self.service.latest.try_take()
    }
}

pub(super) fn publish(
    runtime: Runtime,
    revision: &mut u32,
    measurements: Measurements,
    status: Status,
    orientation: Orientation,
    mag_status: MagStatus,
    mag_field_ut: f32,
    mag_calibration_percent: u8,
) {
    *revision = revision.wrapping_add(1);
    runtime.service.latest.signal(Sample {
        revision: *revision,
        acceleration_m_s2: measurements.acceleration_m_s2,
        angular_velocity_deg_s: measurements.angular_velocity_deg_s,
        magnetic_field_ut: measurements.magnetic_field_ut,
        status,
        orientation,
        mag_status,
        mag_field_strength_ut: mag_field_ut,
        mag_calibration_percent,
    });
}
