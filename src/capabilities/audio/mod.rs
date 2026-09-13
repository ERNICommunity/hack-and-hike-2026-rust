//! Microphone and speaker.
//!
//! Both share one I2S peripheral and clock domain on CPU1: the speaker path is
//! the clock master and the microphone follows it. Applications see two
//! independent handles, [`Microphone`] and [`Speaker`], that exchange signed
//! 16-bit interleaved stereo PCM (left, right, left, right, ...) at
//! [`SAMPLE_RATE_HZ`].

mod capture;
mod channels;
mod codecs;
mod microphone;
mod playback;
mod speaker;

use esp_hal::peripherals::{DMA_CH0, GPIO0, GPIO13, GPIO14, GPIO33, GPIO34, I2S0};

pub use microphone::{MicBlockInfo, Microphone};
pub use speaker::Speaker;

pub(crate) use capture::capture_task;
pub(crate) use channels::{Endpoints, Runtime, endpoints};
pub(crate) use codecs::init_codecs;

/// Sample rate of both the microphone and the speaker.
pub const SAMPLE_RATE_HZ: u32 = 16_000;
/// Left and right.
pub const CHANNELS: usize = 2;
/// Stereo frames in one microphone block (32 ms at 16 kHz).
pub const FRAMES_PER_BLOCK: usize = 512;
/// Samples in one microphone block: `FRAMES_PER_BLOCK` frames of `CHANNELS`.
pub const SAMPLES_PER_BLOCK: usize = FRAMES_PER_BLOCK * CHANNELS;

/// CPU1-owned physical resources of the shared audio runtime.
pub(crate) struct Resources {
    pub(crate) i2s0: I2S0<'static>,
    pub(crate) dma: DMA_CH0<'static>,
    pub(crate) mclk: GPIO0<'static>,
    pub(crate) bclk: GPIO34<'static>,
    pub(crate) word_select: GPIO33<'static>,
    pub(crate) data_in: GPIO14<'static>,
    pub(crate) data_out: GPIO13<'static>,
}
