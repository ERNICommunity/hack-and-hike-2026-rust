//! Sound helpers for the speaker.
//!
//! The speaker plays signed 16-bit samples at [`SAMPLE_RATE_HZ`]. A
//! [`SineWave`] produces them one at a time, so an application can fill the
//! speaker queue a little on every loop iteration:
//!
//! ```ignore
//! let mut tone = SineWave::new(midi_note_hz(69.0)); // A4, 440 Hz
//!
//! // in the loop:
//! let mut chunk = [0i16; 128 * audio::CHANNELS];
//! let frames = speaker.available_frames().min(128);
//! for frame in chunk[..frames * audio::CHANNELS].chunks_exact_mut(audio::CHANNELS) {
//!     frame.fill(tone.next_sample(0.2));
//! }
//! speaker.write(&chunk[..frames * audio::CHANNELS]);
//! ```
//!
//! The little speaker sounds best above about 600 Hz; low notes distort.

use core::f32::consts::TAU;

use crate::capabilities::audio::SAMPLE_RATE_HZ;

/// A sine tone. Call [`SineWave::next_sample`] once per audio frame.
///
/// It is a phase accumulator: every sample advances the phase by a step
/// that depends on the frequency, and the sample is the sine of the phase.
#[derive(Clone, Copy, Debug)]
pub struct SineWave {
    /// Current position in the wave, in radians, `0.0..TAU`.
    phase: f32,
    /// Phase advance per sample, in radians.
    step: f32,
}

impl SineWave {
    /// A tone of `frequency_hz`, starting at phase zero (a silent sample).
    pub fn new(frequency_hz: f32) -> Self {
        Self {
            phase: 0.0,
            step: phase_step(frequency_hz),
        }
    }

    /// Change the pitch without a click: the wave continues from its current
    /// phase.
    pub fn set_frequency(&mut self, frequency_hz: f32) {
        self.step = phase_step(frequency_hz);
    }

    /// The next sample, scaled by `amplitude` (0.0 is silent, 1.0 is the
    /// loudest the speaker can play; 0.2 is plenty).
    pub fn next_sample(&mut self, amplitude: f32) -> i16 {
        let sample = libm::sinf(self.phase) * amplitude.clamp(0.0, 1.0) * f32::from(i16::MAX);
        self.phase = (self.phase + self.step) % TAU;
        sample as i16
    }
}

/// Radians per sample for a tone of `frequency_hz`.
fn phase_step(frequency_hz: f32) -> f32 {
    TAU * frequency_hz / SAMPLE_RATE_HZ as f32
}

/// The frequency of a MIDI note number; 69 is A4 at 440 Hz, 60 is middle C.
/// Fractions transpose by fractions of a semitone.
pub fn midi_note_hz(note: f32) -> f32 {
    440.0 * libm::exp2f((note - 69.0) / 12.0)
}
