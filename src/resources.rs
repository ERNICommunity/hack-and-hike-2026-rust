//! Runtime ownership expressed through concrete resource bundles.
//!
//! These types make the intended architecture visible without trying to prove
//! physical CPU affinity. Bootstrap code is still responsible for moving each
//! bundle to the intended core.
//!
//! - CPU0 owns application/model coordination, Slint presentation, display I/O,
//!   and direct display overlays.
//! - CPU1 owns non-display peripheral services and timing-sensitive acquisition
//!   or communication: touch, audio, future IMU, future ESP-NOW, and runtime
//!   system I2C.
//!
//! Hardware handles should stay inside the resource bundle for their owning
//! side until the corresponding service consumes them.

use esp_hal::peripherals::{
    DMA_CH0, DMA_CH1, GPIO0, GPIO3, GPIO11, GPIO12, GPIO14, GPIO33, GPIO34, GPIO35, GPIO36,
    GPIO37, I2C0, I2S0, SPI2,
};

/// Complete runtime hardware split between the two architectural sides.
pub struct RuntimeResources {
    pub cpu0: Cpu0Resources,
    pub cpu1: Cpu1Resources,
}

/// Resources belonging to CPU0's application/presentation side.
///
/// Application state and Slint do not need raw HAL handles. The concrete
/// hardware here is the display I/O they coordinate.
pub struct Cpu0Resources {
    pub display: DisplayResources,
}

/// CPU0-owned LCD transport resources.
///
/// `screen` becomes the sole runtime owner after construction. LCD controller
/// configuration remains in `screen`.
pub struct DisplayResources {
    pub spi2: SPI2<'static>,
    pub dma: DMA_CH1<'static>,
    pub sck: GPIO36<'static>,
    pub mosi: GPIO37<'static>,
    pub dc: GPIO35<'static>,
    pub cs: GPIO3<'static>,
}

/// Resources belonging to CPU1's non-display peripheral-service side.
///
/// Future IMU and ESP-NOW resources belong here beside system I2C and audio.
pub struct Cpu1Resources {
    pub system_i2c: SystemI2cResources,
    pub audio: AudioResources,
}

/// Physical system-I2C resources whose runtime owner is CPU1.
///
/// The blocking driver is temporarily used during CPU0 startup for board,
/// display, and codec initialization. It is then moved to CPU1 and converted
/// to async for runtime touch/IMU use.
pub struct SystemI2cResources {
    pub i2c0: I2C0<'static>,
    pub sda: GPIO12<'static>,
    pub scl: GPIO11<'static>,
}

/// CPU1-owned I2S microphone acquisition resources.
///
/// ES7210 register configuration remains in `audio`; this type only groups the
/// physical I2S/DMA/GPIO resources belonging to the audio service.
pub struct AudioResources {
    pub i2s0: I2S0<'static>,
    pub dma: DMA_CH0<'static>,
    pub mclk: GPIO0<'static>,
    pub bclk: GPIO34<'static>,
    pub word_select: GPIO33<'static>,
    pub data_in: GPIO14<'static>,
}
