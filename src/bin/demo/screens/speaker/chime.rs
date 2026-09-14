//! A short chime stored in flash as IMA ADPCM and decoded while it plays.
//!
//! `assets/speaker_chime.adpcm` is a mono 16 kHz clip of 11,904 samples,
//! about 744 ms, at four bits per sample.

use hack_and_hike_core::audio::adpcm::Decoder;

/// The encoded clip, embedded in the firmware image at compile time.
const DATA: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/assets/speaker_chime.adpcm"
));
/// Samples in the clip: two per byte.
const CHIME_SAMPLES: usize = 11_904;
const _: () = assert!(DATA.len() * 2 == CHIME_SAMPLES);

/// Plays the chime once per [`FlashChime::restart`].
pub(super) struct FlashChime {
    decoder: Decoder,
    /// Index of the next 4-bit code; two per byte, high nibble first.
    next_code: usize,
    /// Whether the clip is playing.
    playing: bool,
}

impl FlashChime {
    /// A silent chime.
    pub(super) const fn new() -> Self {
        Self {
            decoder: Decoder::new(),
            next_code: 0,
            playing: false,
        }
    }

    /// Play the clip from the start, interrupting a clip in progress.
    pub(super) fn restart(&mut self) {
        self.decoder = Decoder::new();
        self.next_code = 0;
        self.playing = true;
    }

    /// Whether the clip is still playing.
    pub(super) const fn is_playing(&self) -> bool {
        self.playing
    }

    /// The next sample, or silence once the clip has ended.
    pub(super) fn next_sample(&mut self) -> i16 {
        if !self.playing {
            return 0;
        }
        let Some(&byte) = DATA.get(self.next_code / 2) else {
            self.playing = false;
            return 0;
        };
        let code = if self.next_code.is_multiple_of(2) {
            byte >> 4
        } else {
            byte & 0x0F
        };
        self.next_code += 1;
        self.decoder.decode(code)
    }
}
