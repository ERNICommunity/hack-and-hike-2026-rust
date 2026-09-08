//! Cross-core display-control commands.
//!
//! CPU0 presentation produces semantic display intent. CPU1 owns the runtime
//! system-I2C bus and applies that intent to board hardware. Brightness is
//! replace-latest because intermediate slider positions are not meaningful once
//! a newer value exists.

use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use log::warn;

use crate::{board, system_i2c::SystemI2cBus};

static BRIGHTNESS_REQUEST: Signal<CriticalSectionRawMutex, BrightnessPercent> = Signal::new();

/// Valid user-facing LCD brightness percentage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BrightnessPercent(u8);

impl BrightnessPercent {
    pub const OFF: Self = Self(0);
    pub const FULL: Self = Self(100);

    pub const fn new(value: u8) -> Option<Self> {
        if value <= 100 { Some(Self(value)) } else { None }
    }

    pub const fn get(self) -> u8 {
        self.0
    }
}

/// Move-only CPU0 command handle for LCD brightness.
pub struct BrightnessControl {
    _private: (),
}

impl BrightnessControl {
    pub(crate) const fn from_static_service() -> Self {
        Self { _private: () }
    }

    /// Replace any pending brightness request with the newest slider value.
    pub fn set(&mut self, brightness: BrightnessPercent) {
        BRIGHTNESS_REQUEST.signal(brightness);
    }
}

/// CPU1 runtime owner that applies brightness commands over the shared system bus.
#[embassy_executor::task]
pub async fn task(bus: SystemI2cBus) {
    loop {
        let brightness = BRIGHTNESS_REQUEST.wait().await;
        let result = {
            let mut i2c = bus.lock().await;
            board::power::set_lcd_backlight(&mut *i2c, brightness.get()).await
        };

        if let Err(err) = result {
            warn!("LCD brightness update failed: {:?}", err);
        }
    }
}
