//! Ambient light.
//!
//! The LTR-553 sensor behind the front glass measures how bright the
//! surroundings are, ten times a second. A CPU1 task reads it over the shared
//! I2C bus and publishes the newest [`Sample`]. The application takes it
//! with [`Light::latest`]. The same chip also measures proximity. That is the
//! separate [`proximity`](super::proximity) capability, and the same task
//! serves it.
//!
//! The task changes the gain of the sensor by itself: higher in the dark,
//! lower in bright light. After a change, the light data is not valid for a
//! short time, so no light sample is published. The proximity samples
//! continue.
//!
//! ```ignore
//! if let Some(light) = light.as_mut()
//!     && let Some(sample) = light.latest()
//! {
//!     let dark = sample.lux < 10.0;
//! }
//! ```
//!
//! The sensor is optional, like the camera: `Board::init` returns `None`
//! when the sensor does not answer.

mod ltr553;
mod runtime;

use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use log::{info, warn};

use crate::board::registers::Registers;

pub(crate) use runtime::spawn;

/// One ambient light measurement.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    /// Ambient light in lux, 0 or more.
    ///
    /// The sensor is behind the tinted front glass, so the values are lower
    /// than a light meter shows. Use them to compare, not as exact values. A
    /// dark room reads near 0, a room with the lights on reads tens to a few
    /// hundred, and a flashlight pointed at the board reads thousands. Light
    /// that is almost only infrared reads as 0.
    pub lux: f32,
}

/// The state shared by the handle (CPU0) and the light task (CPU1).
struct Service {
    /// The newest sample. A `Signal` holds at most one value: publishing
    /// replaces an unread sample, and taking it leaves the signal empty.
    latest: Signal<CriticalSectionRawMutex, Sample>,
}

/// The one shared light state. A plain `static` is safe to use from both
/// cores, because the signal protects its value with a critical section.
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
    /// previous call. Never waits. Samples come about ten times a second,
    /// with short pauses after a gain change.
    pub fn latest(&mut self) -> Option<Sample> {
        self.service.latest.try_take()
    }
}

/// CPU1 side of the signal, used by the light task.
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

/// Check whether an LTR-553 answers on the bus with the expected part
/// number.
///
/// Bring-up calls this once, before the bus moves to CPU1. It logs the
/// result in both cases. The chip serves both this capability and the
/// proximity capability.
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
