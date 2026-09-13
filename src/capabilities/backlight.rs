//! LCD backlight brightness.
//!
//! The application sets a [`Brightness`]; the CPU1 runtime applies it to the
//! power management chip over the shared I2C bus. Only the newest request
//! matters, so the transport is a replace-latest signal.

use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use log::warn;

use crate::platform::{self, i2c::SystemI2cBus};

/// Backlight brightness in percent, from [`Brightness::MIN`] to
/// [`Brightness::FULL`]. There is no "off": the lowest setting keeps the panel
/// visibly lit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Brightness(u8);

impl Brightness {
    pub const MIN: Self = Self(1);
    pub const FULL: Self = Self(100);

    /// `None` when `percent` is outside `1..=100`.
    pub const fn new(percent: u8) -> Option<Self> {
        if percent >= Self::MIN.0 && percent <= Self::FULL.0 {
            Some(Self(percent))
        } else {
            None
        }
    }

    pub const fn percent(self) -> u8 {
        self.0
    }
}

struct Service {
    request: Signal<CriticalSectionRawMutex, Brightness>,
}

static SERVICE: Service = Service {
    request: Signal::new(),
};

/// Application handle for the LCD backlight.
pub struct Backlight {
    service: &'static Service,
}

impl Backlight {
    /// Request a new brightness. A request that has not been applied yet is
    /// replaced by the newer one.
    pub fn set(&mut self, brightness: Brightness) {
        self.service.request.signal(brightness);
    }
}

#[derive(Clone, Copy)]
pub(crate) struct Runtime {
    service: &'static Service,
}

pub(crate) struct Endpoints {
    pub(crate) handle: Backlight,
    pub(crate) runtime: Runtime,
}

pub(crate) fn endpoints() -> Endpoints {
    Endpoints {
        handle: Backlight { service: &SERVICE },
        runtime: Runtime { service: &SERVICE },
    }
}

/// CPU1 task that applies brightness requests over the shared system bus.
#[embassy_executor::task]
pub(crate) async fn task(bus: SystemI2cBus, runtime: Runtime) {
    loop {
        let brightness = runtime.service.request.wait().await;
        let result = {
            let mut i2c = bus.lock().await;
            platform::power::set_lcd_backlight(&mut *i2c, brightness.percent()).await
        };
        if let Err(error) = result {
            warn!("LCD brightness update failed: {:?}", error);
        }
    }
}
