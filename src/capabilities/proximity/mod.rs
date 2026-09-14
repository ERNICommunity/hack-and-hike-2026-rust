//! Proximity: how close something is to the front of the board.
//!
//! The LTR-553 behind the front glass shines an infrared LED and measures
//! how much comes back, ten times a second. The same chip measures the
//! ambient light; one CPU1 task in the [`light`](super::light) capability
//! reads both and publishes the newest [`Sample`] here. The application
//! takes it with [`Proximity::latest`].
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
//! when none answers.

use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};

/// One proximity measurement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sample {
    /// How close something is, in percent of the range: 0 with nothing
    /// within about 20 cm of the front, 50 at about 10 cm, 100 at the glass.
    /// The scale is even in distance, so a threshold is easy to pick.
    pub percent: u8,
    /// The sensor's own count behind `percent`: how much of its infrared
    /// light comes back, 0 to [`Sample::RAW_MAX`]. It rises with the square
    /// of the closeness, so most of its range lies in the last few
    /// centimetres; useful to see what the sensor really measures.
    pub raw: u16,
}

impl Sample {
    /// The largest raw count: something touches the glass, or the
    /// measurement saturated.
    pub const RAW_MAX: u16 = hack_and_hike_core::light::PROXIMITY_MAX;
}

/// The state shared by the handle (CPU0) and the sensor task (CPU1).
struct Service {
    /// The newest sample. A `Signal` holds at most one value: publishing
    /// replaces an unread sample, and taking it leaves the signal empty.
    latest: Signal<CriticalSectionRawMutex, Sample>,
}

/// The one shared proximity state. A plain `static` works across cores
/// because the signal synchronizes itself.
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
    /// previous call. Samples come ten times a second.
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
