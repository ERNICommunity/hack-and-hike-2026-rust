//! Private physical audio runtime backing the `mic` and `speaker` capability APIs.
//!
//! I2S0 owns the shared microphone/speaker clock domain. The `mic` and `speaker`
//! Cargo capabilities remain independent at the application boundary even though
//! this private runtime shares clocks, DMA, and codec bring-up internally.

mod capture;
mod channels;
mod codecs;
mod playback;

use esp_hal::peripherals::GPIO13;
use esp_hal::peripherals::GPIO14;
use esp_hal::peripherals::{DMA_CH0, GPIO0, GPIO33, GPIO34, I2S0};

pub(crate) use capture::capture_task;
pub(crate) use channels::MicReader;
pub(crate) use channels::SpeakerWriter;
pub(crate) use channels::{Endpoints, Runtime, init_endpoints};
pub(crate) use codecs::init_aw88298;
pub(crate) use codecs::init_es7210;

/// Physical sample rate shared by the I2S0 clock domain.
pub(crate) const SAMPLE_RATE_HZ: u32 = 16_000;

/// CPU1-owned physical resources required by the shared audio runtime.
pub(crate) struct Resources {
    pub(crate) i2s0: I2S0<'static>,
    pub(crate) dma: DMA_CH0<'static>,
    pub(crate) mclk: GPIO0<'static>,
    pub(crate) bclk: GPIO34<'static>,
    pub(crate) word_select: GPIO33<'static>,
    pub(crate) data_in: GPIO14<'static>,
    pub(crate) data_out: GPIO13<'static>,
}
