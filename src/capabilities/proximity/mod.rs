//! Proximity: how close something is to the front of the board.
//!
//! The LTR-553 sensor behind the front glass sends infrared light from an
//! LED and measures how much of it comes back, ten times a second. The same
//! chip measures the ambient light. One CPU1 task in the
//! [`light`](super::light) capability reads both. It publishes the newest
//! [`Sample`] here about every 100 ms, also while the light data is not
//! valid. The application takes it with [`Proximity::latest`].
//!
//! ```ignore
//! if let Some(proximity) = proximity.as_mut()
//!     && let Some(sample) = proximity.latest()
//! {
//!     let covered = sample.percent > 50;
//! }
//! ```
//!
//! The sensor is optional, like the camera: `Board::init` returns `None`
//! when the sensor does not answer.

use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};

/// One proximity measurement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sample {
    /// How close something is, in percent of the range, 0 to 100: 0 when
    /// nothing is within about 20 cm of the front, 50 at about 10 cm, 100 at
    /// the glass. The percentage changes evenly with the distance, so it is
    /// easy to choose a threshold.
    pub percent: u8,
    /// The raw count from the sensor that `percent` is based on: how much of
    /// the infrared light comes back, 0 to [`Sample::RAW_MAX`].
    ///
    /// The count grows with 1 / distance², so most of its range is in the
    /// last few centimetres. It shows what the sensor really measures.
    pub raw: u16,
}

impl Sample {
    /// The largest raw count, 2047. The sensor reports it when something
    /// touches the glass, and when the measurement is saturated (so much
    /// light comes back that the sensor cannot measure more).
    pub const RAW_MAX: u16 = hack_and_hike_core::light::PROXIMITY_MAX;
}

/// The state shared by the handle (CPU0) and the sensor task (CPU1).
struct Service {
    /// The newest sample. A `Signal` holds at most one value: publishing
    /// replaces an unread sample, and taking it leaves the signal empty.
    latest: Signal<CriticalSectionRawMutex, Sample>,
}

/// The one shared proximity state. A plain `static` is safe to use from
/// both cores, because the signal protects its value with a critical
/// section.
static SERVICE: Service = Service {
    latest: Signal::new(),
};

/// Application handle for the proximity sensor; see the [module docs](self).
pub struct Proximity {
    /// Points at the signal shared with the CPU1 sensor task.
    service: &'static Service,
}

impl Proximity {
    /// The newest sample, or `None` when nothing new was published since the
    /// previous call. Never waits. Samples come ten times a second.
    pub fn latest(&mut self) -> Option<Sample> {
        self.service.latest.try_take()
    }
}

/// CPU1 side of the signal, used by the light capability's task.
#[derive(Clone, Copy)]
pub(crate) struct Runtime {
    /// Points at the signal shared with the application's handle on CPU0.
    service: &'static Service,
}

impl Runtime {
    /// Make `sample` the newest one, replacing an unread older sample.
    pub(crate) fn publish(self, sample: Sample) {
        self.service.latest.signal(sample);
    }
}

/// The two ends of the proximity signal, created once by the board.
pub(crate) struct Endpoints {
    /// For the application.
    pub(crate) handle: Proximity,
    /// For the CPU1 task.
    pub(crate) runtime: Runtime,
}

/// Both ends of the proximity signal.
pub(crate) fn endpoints() -> Endpoints {
    Endpoints {
        handle: Proximity { service: &SERVICE },
        runtime: Runtime { service: &SERVICE },
    }
}
