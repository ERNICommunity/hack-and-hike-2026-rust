//! Proximity and ambient light.
//!
//! The LTR-553 on the front of the board measures how bright the surroundings
//! are and whether something is close to the screen, ten times a second. A
//! CPU1 task reads it over the shared I2C bus and publishes the newest
//! [`Sample`]; the application takes it with [`Light::latest`].
//!
//! ```ignore
//! if let Some(light) = light.as_mut()
//!     && let Some(sample) = light.latest()
//! {
//!     let dark = sample.lux < 10.0;
//!     let covered = sample.proximity > 200;
//! }
//! ```
//!
//! The sensor is optional, like the camera: `Board::init` returns `None`
//! when none answers.

mod ltr553;
mod runtime;

use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use log::{info, warn};

use crate::platform::registers::Registers;

pub(crate) use runtime::spawn;

/// One measurement of light and proximity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    /// Ambient light in lux. The sensor sits behind the tinted front glass,
    /// so the values are lower than a light meter would show and are best
    /// used relatively: a dark room reads near 0, a lit room tens to a few
    /// hundred, a torch pointed at the board thousands. Light that is almost
    /// entirely infrared reads as 0.
    pub lux: f32,
    /// How much of the sensor's own infrared light comes back: 0 with
    /// nothing within about 20 cm of the front, more the closer an object
    /// is, up to [`Sample::PROXIMITY_MAX`] when something nearly touches
    /// the glass. The useful threshold depends on the object; try a few
    /// values.
    pub proximity: u16,
}

impl Sample {
    /// The largest proximity count: something touches the sensor, or the
    /// measurement saturated.
    pub const PROXIMITY_MAX: u16 = hack_and_hike_core::light::PROXIMITY_MAX;
}

/// The state shared by the handle (CPU0) and the light task (CPU1).
struct Service {
    /// The newest sample. A `Signal` holds at most one value: publishing
    /// replaces an unread sample, and taking it leaves the signal empty.
    latest: Signal<CriticalSectionRawMutex, Sample>,
}

/// The one shared light state. A plain `static` works across cores because
/// the signal synchronizes itself.
static SERVICE: Service = Service {
    latest: Signal::new(),
};

/// Application handle for the light sensor; see the [module docs](self).
pub struct Light {
    /// Points at the signal shared with the CPU1 light task.
    service: &'static Service,
}

impl Light {
    /// The newest sample, or `None` when nothing new was published since the
    /// previous call. Samples come ten times a second.
    pub fn latest(&mut self) -> Option<Sample> {
        self.service.latest.try_take()
    }
}

/// CPU1 side of the signal.
#[derive(Clone, Copy)]
pub(crate) struct Runtime {
    /// Points at the signal shared with the application's handle on CPU0.
    service: &'static Service,
}

impl Runtime {
    /// Make `sample` the newest one, replacing an unread older sample.
    fn publish(self, sample: Sample) {
        self.service.latest.signal(sample);
    }
}

/// The two ends of the light signal, created once by the board.
pub(crate) struct Endpoints {
    /// For the application.
    pub(crate) handle: Light,
    /// For the CPU1 task.
    pub(crate) runtime: Runtime,
}

/// Both ends of the light signal.
pub(crate) fn endpoints() -> Endpoints {
    Endpoints {
        handle: Light { service: &SERVICE },
        runtime: Runtime { service: &SERVICE },
    }
}

/// Whether an LTR-553 answers on the bus, checked once during bring-up
/// before the bus moves to CPU1. Logs the outcome either way.
pub(crate) fn probe<I2C: embedded_hal::i2c::I2c>(i2c: &mut I2C) -> bool
where
    I2C::Error: core::fmt::Debug,
{
    match Registers::new(i2c, ltr553::ADDRESS).read(ltr553::PART_ID) {
        Ok(id) if id >> 4 == ltr553::PART_NUMBER => {
            info!("LTR-553 light sensor ready (revision {})", id & 0x0F);
            true
        }
        Ok(id) => {
            warn!("No LTR-553 light sensor: unexpected part id {id:#04x}");
            false
        }
        Err(error) => {
            warn!("No LTR-553 light sensor: {error:?}");
            false
        }
    }
}
