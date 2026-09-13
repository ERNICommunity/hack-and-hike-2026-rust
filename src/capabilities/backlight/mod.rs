//! LCD backlight brightness.
//!
//! The application sets a [`Brightness`]; the CPU1 runtime applies it to the
//! power management chip over the shared I2C bus. Only the newest request
//! matters, so the transport is a replace-latest signal.

mod runtime;

use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};

pub(crate) use runtime::spawn;

/// Backlight brightness in percent, from [`Brightness::MIN`] to
/// [`Brightness::FULL`]. There is no "off": the lowest setting keeps the panel
/// visibly lit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Brightness(u8);

/// The percentage was outside 1 to 100.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvalidBrightness;

impl Brightness {
    pub const MIN: Self = Self(1);
    pub const FULL: Self = Self(100);

    /// For percentages written in the code. Panics outside 1 to 100, so a
    /// typo fails at compile time in a `const`.
    pub const fn new(percent: u8) -> Self {
        assert!(
            percent >= Self::MIN.0 && percent <= Self::FULL.0,
            "brightness must be 1 to 100 percent"
        );
        Self(percent)
    }

    pub const fn percent(self) -> u8 {
        self.0
    }
}

/// For percentages computed at run time, for example from a slider.
impl TryFrom<u8> for Brightness {
    type Error = InvalidBrightness;

    fn try_from(percent: u8) -> Result<Self, Self::Error> {
        if (Self::MIN.0..=Self::FULL.0).contains(&percent) {
            Ok(Self(percent))
        } else {
            Err(InvalidBrightness)
        }
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

/// CPU1 side of the signal.
#[derive(Clone, Copy)]
pub(crate) struct Runtime {
    service: &'static Service,
}

impl Runtime {
    async fn next_request(self) -> Brightness {
        self.service.request.wait().await
    }
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
