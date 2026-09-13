//! An eight-note melody played on a sine wave, looping forever.

use hack_and_hike::{
    capabilities::audio::SAMPLE_RATE_HZ,
    synth::{SineWave, midi_note_hz},
};

use super::{PitchSemitones, TempoBpm};

/// The score as MIDI note numbers: a C major scale. It starts at C6 because
/// the tiny speaker distorts on sustained notes much below 600 Hz.
const NOTES: [u8; 8] = [84, 86, 88, 89, 91, 93, 95, 96];
const VOLUME: f32 = 0.15;
/// Every note fades in and out over this many frames, so it starts and ends
/// without a click. Two milliseconds at 16 kHz.
const RAMP_FRAMES: u32 = 32;

pub(super) struct MelodySynth {
    wave: SineWave,
    note: usize,
    frames_into_note: u32,
    /// The pitch the wave was last tuned to, so a slider change retunes it.
    tuned_to: Option<PitchSemitones>,
}

impl MelodySynth {
    pub(super) fn new() -> Self {
        Self {
            wave: SineWave::new(midi_note_hz(f32::from(NOTES[0]))),
            note: 0,
            frames_into_note: 0,
            tuned_to: None,
        }
    }

    pub(super) fn restart(&mut self) {
        self.note = 0;
        self.frames_into_note = 0;
        self.tuned_to = None;
    }

    /// The next audio frame. One note lasts one beat of `tempo`; `pitch`
    /// transposes the whole melody.
    pub(super) fn next_sample(&mut self, tempo: TempoBpm, pitch: PitchSemitones) -> i16 {
        let note_frames = frames_per_beat(tempo);
        if self.frames_into_note >= note_frames {
            self.note = (self.note + 1) % NOTES.len();
            self.frames_into_note = 0;
            self.tuned_to = None;
        }
        if self.tuned_to != Some(pitch) {
            let note = f32::from(NOTES[self.note]) + f32::from(pitch.get());
            self.wave.set_frequency(midi_note_hz(note));
            self.tuned_to = Some(pitch);
        }

        let envelope = ramp(self.frames_into_note, note_frames);
        self.frames_into_note += 1;
        self.wave.next_sample(VOLUME * envelope)
    }
}

fn frames_per_beat(tempo: TempoBpm) -> u32 {
    SAMPLE_RATE_HZ * 60 / u32::from(tempo.get())
}

/// 0.0 at the edges of a note, 1.0 in the middle, with linear ramps.
fn ramp(position: u32, length: u32) -> f32 {
    let rising = position.min(RAMP_FRAMES);
    let falling = length.saturating_sub(position).min(RAMP_FRAMES);
    rising.min(falling) as f32 / RAMP_FRAMES as f32
}
