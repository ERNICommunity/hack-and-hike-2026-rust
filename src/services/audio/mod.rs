//! CPU1-owned physical audio runtime.
//!
//! I2S0 owns the shared microphone/speaker clock domain. The `mic` and `speaker`
//! Cargo capabilities remain independent at the application boundary even though
//! they share clocks, DMA, and codec bring-up internally.

mod capture;
mod channels;
#[cfg(feature = "speaker-synth")]
mod chime;
mod codecs;
#[cfg(feature = "speaker-synth")]
mod melody;
mod playback;

#[cfg(feature = "speaker")]
use esp_hal::peripherals::GPIO13;
#[cfg(feature = "mic")]
use esp_hal::peripherals::GPIO14;
use esp_hal::peripherals::{DMA_CH0, GPIO0, GPIO33, GPIO34, I2S0};

pub(crate) use capture::capture_task;
#[cfg(feature = "mic")]
pub(crate) use channels::Input;
#[cfg(feature = "speaker-synth")]
pub(crate) use channels::PlaybackControl;
pub(crate) use channels::{Endpoints, Runtime, init_endpoints};
pub(crate) use codecs::{init_aw88298, init_es7210};

pub(crate) const SAMPLE_RATE_HZ: u32 = 16_000;
#[cfg(feature = "mic")]
pub(crate) const BLOCK_FRAMES: usize = 512;
#[cfg(feature = "mic")]
pub(crate) const CHANNELS: usize = 2;
#[cfg(feature = "mic")]
pub(crate) const BLOCK_SAMPLES: usize = BLOCK_FRAMES * CHANNELS;

/// Valid melody tempo in quarter-note beats per minute.
#[cfg(feature = "speaker-synth")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TempoBpm(u16);

#[cfg(feature = "speaker-synth")]
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
#[cfg(feature = "speaker-synth")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PitchSemitones(i8);

#[cfg(feature = "speaker-synth")]
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

/// Complete continuous playback intent published by the stock speaker app.
#[cfg(feature = "speaker-synth")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PlaybackSettings {
    pub(crate) melody_playing: bool,
    pub(crate) tempo: TempoBpm,
    pub(crate) pitch: PitchSemitones,
}

#[cfg(feature = "speaker-synth")]
impl PlaybackSettings {
    pub(crate) const DEFAULT: Self = Self {
        melody_playing: false,
        tempo: TempoBpm::DEFAULT,
        pitch: PitchSemitones::CENTER,
    };
}

/// CPU1-owned physical resources required by the shared audio runtime.
pub(crate) struct Resources {
    pub(crate) i2s0: I2S0<'static>,
    pub(crate) dma: DMA_CH0<'static>,
    pub(crate) mclk: GPIO0<'static>,
    pub(crate) bclk: GPIO34<'static>,
    pub(crate) word_select: GPIO33<'static>,
    #[cfg(feature = "mic")]
    pub(crate) data_in: GPIO14<'static>,
    #[cfg(feature = "speaker")]
    pub(crate) data_out: GPIO13<'static>,
}

#[cfg(feature = "mic")]
#[derive(Clone, Copy, Debug)]
pub(crate) struct AudioBlockInfo {
    pub(crate) sequence: u32,
    pub(crate) peak_left: u16,
    pub(crate) peak_right: u16,
}
