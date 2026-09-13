//! Speaker synth application state and PCM generation.

use hack_and_hike::capabilities::speaker::{self, Speaker};

use super::{Action, chime::FlashChime, melody::MelodySynth};

const PCM_FILL_FRAMES: usize = 128;
const PCM_FILL_SAMPLES: usize = PCM_FILL_FRAMES * speaker::CHANNELS;

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SpeakerDisplay {
    pub(crate) melody_playing: bool,
    pub(crate) tempo: TempoBpm,
    pub(crate) pitch: PitchSemitones,
}

impl SpeakerDisplay {
    pub(crate) const DEFAULT: Self = Self {
        melody_playing: false,
        tempo: TempoBpm::DEFAULT,
        pitch: PitchSemitones::CENTER,
    };
}

struct PlaybackEngine {
    melody: MelodySynth,
    chime: FlashChime,
}

impl PlaybackEngine {
    const fn new() -> Self {
        Self {
            melody: MelodySynth::new(),
            chime: FlashChime::new(),
        }
    }

    fn has_audio(&self, state: SpeakerDisplay) -> bool {
        state.melody_playing || self.chime.is_playing()
    }

    fn fill_interleaved(&mut self, samples: &mut [i16], state: SpeakerDisplay) {
        for frame in samples.chunks_exact_mut(speaker::CHANNELS) {
            let melody = if state.melody_playing {
                self.melody.next_sample(state.tempo, state.pitch)
            } else {
                0
            };
            let sample = saturating_mix(melody, self.chime.next_sample());
            frame.fill(sample);
        }
    }
}

pub(crate) struct Model {
    speaker: Speaker,
    display: SpeakerDisplay,
    engine: PlaybackEngine,
    pending: [i16; PCM_FILL_SAMPLES],
    pending_frames: usize,
    pending_offset_frames: usize,
    dirty: bool,
}

impl Model {
    pub(crate) fn new(speaker: Speaker) -> Self {
        Self {
            speaker,
            display: SpeakerDisplay::DEFAULT,
            engine: PlaybackEngine::new(),
            pending: [0; PCM_FILL_SAMPLES],
            pending_frames: 0,
            pending_offset_frames: 0,
            dirty: true,
        }
    }

    pub(crate) fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    pub(crate) fn apply(&mut self, action: Action) {
        match action {
            Action::TogglePlayback => {
                self.display.melody_playing = !self.display.melody_playing;
                if self.display.melody_playing {
                    self.engine.melody.restart();
                }
                self.dirty = true;
            }
            Action::PlayOneShot => self.engine.chime.restart(),
            Action::SetTempo(tempo) => {
                if self.display.tempo != tempo {
                    self.display.tempo = tempo;
                    self.dirty = true;
                }
            }
            Action::SetPitch(pitch) => {
                if self.display.pitch != pitch {
                    self.display.pitch = pitch;
                    self.dirty = true;
                }
            }
        }
    }

    /// Keep the bounded speaker queue fed independently of which screen is active.
    pub(crate) fn update(&mut self) {
        loop {
            if self.pending_offset_frames < self.pending_frames {
                let first_sample = self.pending_offset_frames * speaker::CHANNELS;
                let last_sample = self.pending_frames * speaker::CHANNELS;
                let written = self
                    .speaker
                    .try_write_interleaved(&self.pending[first_sample..last_sample]);
                if written == 0 {
                    return;
                }
                self.pending_offset_frames += written;
                if self.pending_offset_frames < self.pending_frames {
                    return;
                }
                self.pending_frames = 0;
                self.pending_offset_frames = 0;
            }

            if !self.engine.has_audio(self.display) {
                return;
            }

            let frames = self.speaker.available_frames().min(PCM_FILL_FRAMES);
            if frames == 0 {
                return;
            }

            let sample_count = frames * speaker::CHANNELS;
            self.engine
                .fill_interleaved(&mut self.pending[..sample_count], self.display);
            self.pending_frames = frames;
            self.pending_offset_frames = 0;
        }
    }

    pub(crate) fn take_display(&mut self) -> Option<SpeakerDisplay> {
        if !self.dirty {
            return None;
        }
        self.dirty = false;
        Some(self.display)
    }
}

fn saturating_mix(a: i16, b: i16) -> i16 {
    i32::from(a)
        .saturating_add(i32::from(b))
        .clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16
}
