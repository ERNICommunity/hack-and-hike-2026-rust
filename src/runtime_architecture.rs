//! Runtime architecture expressed through concrete ownership-oriented types.
//!
//! This module intentionally does not use CPU capability tokens. Rust does not
//! prove which physical core is executing a function here. Instead, the type
//! system makes the intended architecture explicit by grouping raw HAL handles
//! into verbose, core-owned resource bundles and by requiring services to
//! accept those bundles instead of unrelated individual peripherals.
//!
//! The intended split is:
//!
//! - CPU0: application/model coordination, Slint presentation, LCD, SPI2,
//!   display DMA, and direct display overlays.
//! - CPU1: non-display peripheral services and timing-sensitive acquisition or
//!   communication: touch, audio, future IMU, future ESP-NOW, and runtime
//!   system I2C.
//!
//! The system I2C hardware is configured in blocking mode during CPU0 startup
//! because board/display/audio initialization needs it before CPU1 starts. The
//! configured driver is then moved to CPU1, converted to async there, and used
//! as CPU1's runtime system bus.

use esp_hal::peripherals::{
    DMA_CH0, DMA_CH1, GPIO0, GPIO3, GPIO11, GPIO12, GPIO14, GPIO33, GPIO34, GPIO35, GPIO36,
    GPIO37, I2C0, I2S0, SPI2,
};

/// Top-level runtime hardware split.
///
/// Reading this type should be enough to recover the core ownership model
/// without consulting a separate architecture document.
pub struct RuntimeArchitectureResources {
    pub cpu0_application_presentation_and_display:
        Cpu0ApplicationPresentationAndDisplayResources,
    pub cpu1_non_display_peripheral_services:
        Cpu1NonDisplayPeripheralServicesResources,
}

/// Hardware that belongs to CPU0's application/presentation/display side.
///
/// Application models and Slint do not need raw HAL handles, so the concrete
/// hardware represented here is the display I/O they coordinate.
pub struct Cpu0ApplicationPresentationAndDisplayResources {
    pub display_io: Cpu0DisplayIoResources,
}

/// CPU0-owned LCD transport resources.
///
/// `screen` is the sole runtime owner of these handles after construction.
/// Display-controller configuration also remains in `screen`.
pub struct Cpu0DisplayIoResources {
    pub spi2: SPI2<'static>,
    pub display_dma: DMA_CH1<'static>,
    pub display_sck: GPIO36<'static>,
    pub display_mosi: GPIO37<'static>,
    pub display_dc: GPIO35<'static>,
    pub display_cs: GPIO3<'static>,
}

/// Hardware that belongs to CPU1's non-display peripheral-service side.
///
/// Touch shares `runtime_system_i2c`; audio owns the I2S acquisition hardware.
/// Future BMI270/IMU and ESP-NOW resources belong in this type rather than in
/// the CPU0 bundle.
pub struct Cpu1NonDisplayPeripheralServicesResources {
    pub runtime_system_i2c: Cpu1RuntimeSystemI2cResources,
    pub audio_acquisition: Cpu1AudioAcquisitionResources,
}

/// Physical system-I2C resources whose runtime owner is CPU1.
///
/// They are initially consumed by `system_i2c::init()` on CPU0 for one-time
/// board startup. The resulting blocking driver is later moved to CPU1 and
/// converted to async there.
pub struct Cpu1RuntimeSystemI2cResources {
    pub i2c0: I2C0<'static>,
    pub system_i2c_sda: GPIO12<'static>,
    pub system_i2c_scl: GPIO11<'static>,
}

/// CPU1-owned I2S microphone acquisition resources.
///
/// ES7210 register configuration remains in `audio`; this type only describes
/// which physical I2S/DMA/GPIO resources belong to the CPU1 audio service.
pub struct Cpu1AudioAcquisitionResources {
    pub i2s0: I2S0<'static>,
    pub audio_dma: DMA_CH0<'static>,
    pub microphone_mclk: GPIO0<'static>,
    pub microphone_bclk: GPIO34<'static>,
    pub microphone_word_select: GPIO33<'static>,
    pub microphone_data_in: GPIO14<'static>,
}
