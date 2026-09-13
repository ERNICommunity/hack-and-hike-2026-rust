//! CPU1-to-CPU0 IMU latest-value transport.

use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use static_cell::StaticCell;

use super::{Measurements, Orientation, Sample, Status, magnetic::MagneticReport};

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
pub struct Imu {
    service: &'static Service,
}

pub(crate) struct Endpoints {
    pub runtime: Runtime,
    pub input: Imu,
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
    pub fn latest(&mut self) -> Option<Sample> {
        self.service.latest.try_take()
    }
}

/// CPU1 side of the transport: numbers and publishes samples.
pub(super) struct Publisher {
    runtime: Runtime,
    revision: u32,
}

impl Publisher {
    pub(super) const fn new(runtime: Runtime) -> Self {
        Self {
            runtime,
            revision: 0,
        }
    }

    /// Revision carried by the most recently published sample.
    pub(super) const fn revision(&self) -> u32 {
        self.revision
    }

    /// Revision the next published sample will carry.
    pub(super) const fn next_revision(&self) -> u32 {
        self.revision.wrapping_add(1)
    }

    pub(super) fn publish(
        &mut self,
        status: Status,
        measurements: Measurements,
        orientation: Orientation,
        magnetic: MagneticReport,
    ) {
        self.revision = self.next_revision();
        self.runtime.service.latest.signal(Sample {
            revision: self.revision,
            acceleration_m_s2: measurements.acceleration_m_s2,
            angular_velocity_deg_s: measurements.angular_velocity_deg_s,
            magnetic_field_ut: measurements.magnetic_field_ut,
            status,
            orientation,
            mag_status: magnetic.status,
            mag_field_strength_ut: magnetic.field_ut,
            mag_calibration_percent: magnetic.calibration_percent,
        });
    }
}
