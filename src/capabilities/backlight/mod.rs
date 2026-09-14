//! LCD backlight brightness.
//!
//! The application sets a [`Brightness`]; a CPU1 task applies it to the power
//! chip over the shared I2C bus. Only the newest request matters, so a
//! request that has not been applied yet is replaced by a newer one.
//!
//! ```ignore
//! backlight.set(Brightness::new(30));
//! ```

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
    /// The dimmest setting, 1 %. The panel stays readable.
    pub const MIN: Self = Self(1);
    /// Full brightness, 100 %: the setting at boot.
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

    /// The brightness in percent, 1 to 100.
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

/// The state shared by the handle (CPU0) and the backlight task (CPU1).
struct Service {
    /// The newest brightness not yet applied. A `Signal` holds at most one value:
    /// signalling again overwrites it, which is exactly "only the newest request
    /// matters".
    request: Signal<CriticalSectionRawMutex, Brightness>,
}

/// The one shared backlight state. A plain `static` works across cores because
/// the signal synchronizes itself.
static SERVICE: Service = Service {
    request: Signal::new(),
};

/// Application handle for the LCD backlight; see the [module docs](self).
pub struct Backlight {
    /// Points at the signal shared with the CPU1 backlight task.
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
    /// Points at the signal shared with the application's handle on CPU0.
    service: &'static Service,
}

impl Runtime {
    /// Wait for the next brightness request.
    async fn next_request(self) -> Brightness {
        self.service.request.wait().await
    }
}

/// The two ends of the brightness signal, created once by the board.
pub(crate) struct Endpoints {
    /// For the application.
    pub(crate) handle: Backlight,
    /// For the CPU1 task.
    pub(crate) runtime: Runtime,
}

/// Both ends of the brightness signal.
pub(crate) fn endpoints() -> Endpoints {
    Endpoints {
        handle: Backlight { service: &SERVICE },
        runtime: Runtime { service: &SERVICE },
    }
}
