//! The brightness of the screen's backlight.
//!
//! The application sets a [`Brightness`]. A CPU1 task sends it to the power
//! chip over the shared I2C bus. Only the newest request matters: a new
//! request replaces an older request that is not applied yet.
//!
//! ```ignore
//! backlight.set(Brightness::new(30));
//! ```

mod runtime;

use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};

pub(crate) use runtime::spawn;

/// Backlight brightness in percent, from [`Brightness::MIN`] to
/// [`Brightness::FULL`].
///
/// There is no "off": at the lowest setting, the screen is still lit. The
/// power chip has only eight brightness levels, so percentages that are
/// close to each other can look the same.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Brightness(u8);

/// Error: the percentage is not between 1 and 100.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvalidBrightness;

impl Brightness {
    /// The lowest setting, 1 %. The screen is still readable.
    pub const MIN: Self = Self(1);
    /// Full brightness, 100 %. This is the setting at start-up.
    pub const FULL: Self = Self(100);

    /// A brightness of `percent`, for a value written in the code. For a
    /// value computed at run time, use `Brightness::try_from`.
    ///
    /// # Panics
    ///
    /// When `percent` is not between 1 and 100. In a `const`, the error
    /// appears at compile time.
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

/// For a percentage computed at run time, for example from a slider.
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
    /// The newest brightness that is not applied yet. A `Signal` holds at
    /// most one value. A new request replaces the old one, so only the newest
    /// request is applied.
    request: Signal<CriticalSectionRawMutex, Brightness>,
}

/// The one shared backlight state. A plain `static` is safe to use from both
/// cores, because the signal protects its value with a critical section.
static SERVICE: Service = Service {
    request: Signal::new(),
};

/// Application handle for the LCD backlight; see the [module docs](self).
pub struct Backlight {
    /// Points at the signal shared with the CPU1 backlight task.
    service: &'static Service,
}

impl Backlight {
    /// Request a new brightness. Never waits. When an earlier request is not
    /// applied yet, this request replaces it.
    pub fn set(&mut self, brightness: Brightness) {
        self.service.request.signal(brightness);
    }
}

/// CPU1 side of the signal, used by the backlight task.
#[derive(Clone, Copy)]
pub(crate) struct Runtime {
    /// Points at the signal shared with the application's handle on CPU0.
    service: &'static Service,
}

impl Runtime {
    /// Wait for the next brightness request, and take it out of the signal.
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
