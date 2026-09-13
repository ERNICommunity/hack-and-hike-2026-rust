//! Sound helpers for the speaker.
//!
//! The speaker plays signed 16-bit samples at [`SAMPLE_RATE_HZ`]. A
//! [`SineWave`] produces them one at a time, so an application can fill the
//! speaker queue a little on every loop iteration.

use core::f32::consts::TAU;

use crate::capabilities::audio::SAMPLE_RATE_HZ;

/// A sine tone. Call [`SineWave::next_sample`] once per audio frame.
#[derive(Clone, Copy, Debug)]
pub struct SineWave {
    phase: f32,
    step: f32,
}

impl SineWave {
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

fn phase_step(frequency_hz: f32) -> f32 {
    TAU * frequency_hz / SAMPLE_RATE_HZ as f32
}

/// The frequency of a MIDI note number; 69 is A4 at 440 Hz, 60 is middle C.
/// Fractions transpose by fractions of a semitone.
pub fn midi_note_hz(note: f32) -> f32 {
    440.0 * libm::exp2f((note - 69.0) / 12.0)
}
