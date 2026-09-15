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
//! The small speaker sounds best above about 600 Hz. Lower notes sound
//! distorted.

use core::f32::consts::TAU;

use crate::capabilities::audio::SAMPLE_RATE_HZ;

/// A sine tone. Call [`SineWave::next_sample`] once per audio frame.
///
/// It works as a phase accumulator: every sample moves the phase forward by
/// a step that depends on the frequency. The sample is the sine of the phase.
#[derive(Clone, Copy, Debug)]
pub struct SineWave {
    /// Current position in the wave, in radians, `0.0..TAU`.
    phase: f32,
    /// How far the phase moves forward with each sample, in radians.
    step: f32,
}

impl SineWave {
    /// A tone of `frequency_hz` hertz. The wave starts at phase zero, so the
    /// first sample is 0.
    pub fn new(frequency_hz: f32) -> Self {
        Self {
            phase: 0.0,
            step: phase_step(frequency_hz),
        }
    }

    /// Change the frequency. The wave continues from its current phase, so
    /// the change makes no click.
    pub fn set_frequency(&mut self, frequency_hz: f32) {
        self.step = phase_step(frequency_hz);
    }

    /// Return the next sample, scaled by `amplitude`.
    ///
    /// 0.0 is silent, and 1.0 is the loudest sample the speaker can play.
    /// Values outside 0.0 to 1.0 are clamped. 0.2 is loud enough for most
    /// uses.
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

/// The frequency in hertz of a MIDI note number.
///
/// MIDI numbers the notes of a piano keyboard: 69 is A4 at 440 Hz, and 60 is
/// middle C. One step is one semitone. A fraction gives a pitch between two
/// semitones.
pub fn midi_note_hz(note: f32) -> f32 {
    440.0 * libm::exp2f((note - 69.0) / 12.0)
}
