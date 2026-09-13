//! Cross-core display-control commands.
//!
//! CPU0 presentation produces semantic display intent. CPU1 owns the runtime
//! system-I2C bus and applies that intent to board hardware. Brightness is
//! replace-latest because intermediate slider positions are not meaningful once
//! a newer value exists.

use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use log::warn;
use static_cell::StaticCell;

use crate::platform::{self, i2c::SystemI2cBus};

/// Valid user-facing LCD brightness percentage.
///
/// Runtime brightness intentionally has no OFF state. The lowest setting keeps
/// the panel visibly powered; display power policy is separate from dimming.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Brightness(u8);

impl Brightness {
    pub const MIN: Self = Self(1);
    pub const FULL: Self = Self(100);

    pub const fn new(value: u8) -> Option<Self> {
        if value >= Self::MIN.0 && value <= Self::FULL.0 {
            Some(Self(value))
        } else {
            None
        }
    }

    pub const fn get(self) -> u8 {
        self.0
    }
}

type RequestSignal = Signal<CriticalSectionRawMutex, Brightness>;

struct Service {
    request: RequestSignal,
}

impl Service {
    const fn new() -> Self {
        Self {
            request: Signal::new(),
        }
    }
}

static SERVICE: StaticCell<Service> = StaticCell::new();

#[derive(Clone, Copy)]
pub(crate) struct Runtime {
    service: &'static Service,
}

/// Move-only CPU0 command handle for LCD brightness.
pub struct BrightnessControl {
    service: &'static Service,
}

pub(crate) struct Endpoints {
    pub runtime: Runtime,
    pub control: BrightnessControl,
}

pub(crate) fn init_endpoints() -> Endpoints {
    let service: &'static Service = SERVICE.init(Service::new());
    Endpoints {
        runtime: Runtime { service },
        control: BrightnessControl { service },
    }
}

impl BrightnessControl {
    /// Replace any pending brightness request with the newest slider value.
    pub fn set(&mut self, brightness: Brightness) {
        self.service.request.signal(brightness);
    }
}

/// CPU1 runtime owner that applies brightness commands over the shared system bus.
#[embassy_executor::task]
pub(crate) async fn task(bus: SystemI2cBus, runtime: Runtime) {
    loop {
        let brightness = runtime.service.request.wait().await;
        let result = {
            let mut i2c = bus.lock().await;
            platform::power::set_lcd_backlight(&mut *i2c, brightness.get()).await
        };

        if let Err(err) = result {
            warn!("LCD brightness update failed: {:?}", err);
        }
    }
}
