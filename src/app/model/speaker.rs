//! Speaker presentation and playback-control model.

use crate::audio;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpeakerDisplay {
    pub melody_playing: bool,
    pub tempo: audio::TempoBpm,
    pub pitch: audio::PitchSemitones,
}

impl SpeakerDisplay {
    pub const DEFAULT: Self = Self {
        melody_playing: false,
        tempo: audio::TempoBpm::DEFAULT,
        pitch: audio::PitchSemitones::CENTER,
    };
}

pub(super) struct Model {
    control: audio::PlaybackControl,
    display: SpeakerDisplay,
    dirty: bool,
}

impl Model {
    pub(super) fn new(control: audio::PlaybackControl) -> Self {
        Self {
            control,
            display: SpeakerDisplay::DEFAULT,
            dirty: true,
        }
    }

    pub(super) fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    fn publish(&mut self) {
        self.control.set(audio::PlaybackSettings {
            melody_playing: self.display.melody_playing,
            tempo: self.display.tempo,
            pitch: self.display.pitch,
        });
        self.dirty = true;
    }

    pub(super) fn toggle_playback(&mut self) {
        self.display.melody_playing = !self.display.melody_playing;
        self.publish();
    }

    pub(super) fn set_tempo(&mut self, tempo: audio::TempoBpm) {
        if self.display.tempo != tempo {
            self.display.tempo = tempo;
            self.publish();
        }
    }

    pub(super) fn set_pitch(&mut self, pitch: audio::PitchSemitones) {
        if self.display.pitch != pitch {
            self.display.pitch = pitch;
            self.publish();
        }
    }

    pub(super) fn play_one_shot(&mut self) {
        self.control.play_one_shot();
    }

    pub(super) fn take_display(&mut self) -> Option<SpeakerDisplay> {
        if !self.dirty {
            return None;
        }
        self.dirty = false;
        Some(self.display)
    }
}
