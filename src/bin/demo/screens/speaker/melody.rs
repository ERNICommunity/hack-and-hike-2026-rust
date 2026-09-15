//! An eight-note melody played on a sine wave. It repeats forever.
//!
//! Every note lasts one beat. The synthesizer counts frames to know when the
//! next note starts. Then it changes the frequency of its [`SineWave`]. It
//! also changes the frequency when the pitch slider moves.

use hack_and_hike::{
    capabilities::audio::SAMPLE_RATE_HZ,
    synth::{SineWave, midi_note_hz},
};

use super::{PitchSemitones, TempoBpm};

/// The notes as MIDI note numbers: a C major scale. (MIDI numbers the notes
/// in semitones; 69 is A4 at 440 Hz.) The scale starts at C6 (84, about
/// 1047 Hz), because the small speaker distorts long notes much below
/// 600 Hz.
const NOTES: [u8; 8] = [84, 86, 88, 89, 91, 93, 95, 96];
/// Melody loudness, 0.0 to 1.0.
const VOLUME: f32 = 0.15;
/// Every note fades in and out over this many frames, so it starts and ends
/// without a click. 32 frames are 2 ms at 16 kHz.
const RAMP_FRAMES: u32 = 32;

/// Plays [`NOTES`] in a loop, one frame at a time.
pub(super) struct MelodySynth {
    /// The oscillator of the current note.
    wave: SineWave,
    /// Index of the current note in [`NOTES`].
    note: usize,
    /// Frames of the current note already played.
    frames_into_note: u32,
    /// The transposition that the wave frequency was last set for. When the
    /// slider changes, the frequency is set again. `None` forces a new
    /// frequency on the next sample.
    tuned_to: Option<PitchSemitones>,
}

impl MelodySynth {
    /// A synthesizer at the first note.
    pub(super) fn new() -> Self {
        Self {
            wave: SineWave::new(midi_note_hz(f32::from(NOTES[0]))),
            note: 0,
            frames_into_note: 0,
            tuned_to: None,
        }
    }

    /// Go back to the first note.
    pub(super) fn restart(&mut self) {
        self.note = 0;
        self.frames_into_note = 0;
        self.tuned_to = None;
    }

    /// The next sample: one sample for each audio frame. One note lasts one
    /// beat of `tempo`. `pitch` transposes the whole melody.
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

/// How many audio frames one beat lasts at `tempo`.
fn frames_per_beat(tempo: TempoBpm) -> u32 {
    SAMPLE_RATE_HZ * 60 / u32::from(tempo.get())
}

/// The volume factor at `position` in a note of `length` frames: it rises
/// linearly from 0.0 to 1.0 over `RAMP_FRAMES` at the start, and falls to
/// 0.0 the same way at the end.
fn ramp(position: u32, length: u32) -> f32 {
    let rising = position.min(RAMP_FRAMES);
    let falling = length.saturating_sub(position).min(RAMP_FRAMES);
    rising.min(falling) as f32 / RAMP_FRAMES as f32
}
