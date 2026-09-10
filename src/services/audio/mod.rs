//! CPU1-owned full-duplex audio service.
//!
//! I2S0 owns the shared microphone/speaker clock domain. RX continuously captures
//! the ES7210 microphones while TX continuously feeds the AW88298 amplifier.
//! CPU0 sees separate microphone-input and speaker-control endpoints; DMA,
//! synchronization, codec registers, and synthesis remain private here.

mod channels;
mod chime;
mod codecs;
mod capture;
mod melody;
mod playback;

use esp_hal::peripherals::{DMA_CH0, GPIO0, GPIO13, GPIO14, GPIO33, GPIO34, I2S0};

pub(crate) use channels::{Input, PlaybackControl};
pub(crate) use codecs::{init_aw88298, init_es7210};
pub(crate) use capture::capture_task;
pub(crate) use channels::{Endpoints, Runtime, init_endpoints};

pub(crate) const SAMPLE_RATE_HZ: u32 = 16_000;
pub(crate) const BLOCK_FRAMES: usize = 512;
pub(crate) const CHANNELS: usize = 2;
pub(crate) const BLOCK_SAMPLES: usize = BLOCK_FRAMES * CHANNELS;

/// Valid melody tempo in quarter-note beats per minute.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TempoBpm(u16);

impl TempoBpm {
    pub(crate) const MIN: Self = Self(60);
    pub(crate) const DEFAULT: Self = Self(120);
    pub(crate) const MAX: Self = Self(180);

    pub(crate) const fn new(value: u16) -> Option<Self> {
        if value >= Self::MIN.0 && value <= Self::MAX.0 {
            Some(Self(value))
        } else {
            None
        }
    }

    pub(crate) const fn get(self) -> u16 {
        self.0
    }
}

/// Chromatic pitch transposition applied to the synthesized MIDI loop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PitchSemitones(i8);

impl PitchSemitones {
    pub(crate) const MIN: Self = Self(-12);
    pub(crate) const CENTER: Self = Self(0);
    pub(crate) const MAX: Self = Self(12);

    pub(crate) const fn new(value: i8) -> Option<Self> {
        if value >= Self::MIN.0 && value <= Self::MAX.0 {
            Some(Self(value))
        } else {
            None
        }
    }

    pub(crate) const fn get(self) -> i8 {
        self.0
    }
}

/// Complete continuous playback intent published by CPU0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PlaybackSettings {
    pub(crate) melody_playing: bool,
    pub(crate) tempo: TempoBpm,
    pub(crate) pitch: PitchSemitones,
}

impl PlaybackSettings {
    pub(crate) const DEFAULT: Self = Self {
        melody_playing: false,
        tempo: TempoBpm::DEFAULT,
        pitch: PitchSemitones::CENTER,
    };
}

/// CPU1-owned physical resources required by the shared audio service.
pub(crate) struct Resources {
    pub(crate) i2s0: I2S0<'static>,
    pub(crate) dma: DMA_CH0<'static>,
    pub(crate) mclk: GPIO0<'static>,
    pub(crate) bclk: GPIO34<'static>,
    pub(crate) word_select: GPIO33<'static>,
    pub(crate) data_in: GPIO14<'static>,
    pub(crate) data_out: GPIO13<'static>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct AudioBlockInfo {
    pub(crate) sequence: u32,
    pub(crate) peak_left: u16,
    pub(crate) peak_right: u16,
}
