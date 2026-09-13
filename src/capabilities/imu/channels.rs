//! CPU1-to-CPU0 transport of IMU samples.

use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};

use super::{Measurements, Orientation, Sample, Status, magnetic::MagneticReport};

struct Service {
    latest: Signal<CriticalSectionRawMutex, Sample>,
}

static SERVICE: Service = Service {
    latest: Signal::new(),
};

/// Application handle for the motion sensors.
///
/// CPU1 publishes about 100 samples per second. Only the newest one is kept,
/// so an application that polls slower than that always sees fresh data and
/// can detect skipped samples through [`Sample::revision`].
pub struct Imu {
    service: &'static Service,
}

impl Imu {
    /// The newest sample, or `None` when nothing new was published since the
    /// previous call.
    pub fn latest(&mut self) -> Option<Sample> {
        self.service.latest.try_take()
    }
}

#[derive(Clone, Copy)]
pub(crate) struct Runtime {
    service: &'static Service,
}

pub(crate) struct Endpoints {
    pub(crate) handle: Imu,
    pub(crate) runtime: Runtime,
}

pub(crate) fn endpoints() -> Endpoints {
    Endpoints {
        handle: Imu { service: &SERVICE },
        runtime: Runtime { service: &SERVICE },
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

    pub(super) fn publish(
        &mut self,
        status: Status,
        measurements: Measurements,
        orientation: Orientation,
        magnetic: MagneticReport,
    ) {
        self.revision = self.revision.wrapping_add(1);
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
