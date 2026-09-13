//! Melody synthesizer and chime playback with tempo and pitch controls.
//!
//! Audio keeps playing while another screen is visible, so the speaker queue
//! is fed from `update`, which the shell calls for every screen.

mod chime;
mod melody;

use embedded_gui::WidgetId;
use hack_and_hike::{
    capabilities::{
        audio::{self, Speaker},
        display::Surface,
    },
    ui::{
        gui::{self, GuiSurface, Pointer},
        widgets::Slider,
    },
};

use crate::{layout, screens::Screen, styles};

use chime::FlashChime;
use melody::MelodySynth;

mod generated {
    use embedded_gui::prelude::*;
    embedded_gui::include_gui!("src/bin/demo/screens/speaker/speaker.kdl");
}

const NODES: usize = 16;
const PCM_CHUNK_FRAMES: usize = 128;
const PCM_CHUNK_SAMPLES: usize = PCM_CHUNK_FRAMES * audio::CHANNELS;

/// Melody tempo in quarter-note beats per minute.
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

/// Transposition of the melody in semitones.
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

pub(crate) struct SpeakerScreen {
    speaker: Speaker,
    melody: MelodySynth,
    chime: FlashChime,
    playing: bool,
    tempo: TempoBpm,
    pitch: PitchSemitones,
    gui: &'static mut gui::Context<NODES>,
    play_button: WidgetId,
    chime_button: WidgetId,
    status_label: WidgetId,
    tempo_value: WidgetId,
    pitch_value: WidgetId,
    tempo_slider: Slider,
    pitch_slider: Slider,
    dirty: bool,
}

impl SpeakerScreen {
    pub(crate) fn new(speaker: Speaker) -> Self {
        let gui = gui::context::<NODES>(layout::CONTENT_WIDTH, layout::CONTENT_HEIGHT);
        let app = generated::SpeakerApp::build(gui).expect("speaker.kdl fits the GUI capacities");
        let tempo_value = gui
            .add_value_label(
                gui::slot(gui, app.widgets.tempo_value),
                "TEMPO BPM",
                i32::from(TempoBpm::DEFAULT.get()),
                styles::value(),
            )
            .expect("room for the tempo value");
        let pitch_value = gui
            .add_value_label(
                gui::slot(gui, app.widgets.pitch_value),
                "PITCH SEMITONES",
                i32::from(PitchSemitones::CENTER.get()),
                styles::value(),
            )
            .expect("room for the pitch value");

        Self {
            speaker,
            melody: MelodySynth::new(),
            chime: FlashChime::new(),
            playing: false,
            tempo: TempoBpm::DEFAULT,
            pitch: PitchSemitones::CENTER,
            play_button: app.widgets.play,
            chime_button: app.widgets.chime,
            status_label: app.widgets.status,
            tempo_value,
            pitch_value,
            tempo_slider: Slider::new(
                gui::slot(gui, app.widgets.tempo_slider),
                i32::from(TempoBpm::MIN.get()),
                i32::from(TempoBpm::MAX.get()),
            ),
            pitch_slider: Slider::new(
                gui::slot(gui, app.widgets.pitch_slider),
                i32::from(PitchSemitones::MIN.get()),
                i32::from(PitchSemitones::MAX.get()),
            ),
            gui,
            dirty: true,
        }
    }

    fn next_sample(&mut self) -> i16 {
        let melody = if self.playing {
            self.melody.next_sample(self.tempo, self.pitch)
        } else {
            0
        };
        i32::from(melody)
            .saturating_add(i32::from(self.chime.next_sample()))
            .clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16
    }

    /// Generate as much audio as the speaker queue accepts right now.
    fn feed_speaker(&mut self) {
        while self.playing || self.chime.is_playing() {
            let frames = self.speaker.available_frames().min(PCM_CHUNK_FRAMES);
            if frames == 0 {
                return;
            }
            let mut pcm = [0i16; PCM_CHUNK_SAMPLES];
            for frame in pcm[..frames * audio::CHANNELS].chunks_exact_mut(audio::CHANNELS) {
                frame.fill(self.next_sample());
            }
            self.speaker.write(&pcm[..frames * audio::CHANNELS]);
        }
    }
}

impl Screen for SpeakerScreen {
    fn enter(&mut self) {
        self.dirty = true;
    }

    fn update(&mut self, _now: embassy_time::Instant) {
        self.feed_speaker();
    }

    fn handle_pointer(&mut self, pointer: Pointer) {
        let mut clicked = None;
        gui::click_buttons(self.gui, pointer, |id| clicked = Some(id));
        if clicked == Some(self.play_button) {
            self.playing = !self.playing;
            if self.playing {
                self.melody.restart();
            }
        } else if clicked == Some(self.chime_button) {
            self.chime.restart();
        }

        if let Some(tempo) = self
            .tempo_slider
            .handle_pointer(pointer)
            .and_then(|value| u16::try_from(value).ok())
            .and_then(TempoBpm::new)
        {
            self.tempo = tempo;
        }
        if let Some(pitch) = self
            .pitch_slider
            .handle_pointer(pointer)
            .and_then(|value| i8::try_from(value).ok())
            .and_then(PitchSemitones::new)
        {
            self.pitch = pitch;
        }
        // Buttons change appearance while pressed, so redraw on every touch.
        self.dirty = true;
    }

    fn present(&mut self, gui: &mut GuiSurface, surface: &mut Surface<'_>) {
        if !self.dirty {
            return;
        }
        self.dirty = false;

        let (button, status) = if self.playing {
            ("STOP", "MELODY PLAYING")
        } else {
            ("PLAY", "MELODY STOPPED")
        };
        gui::set_text(self.gui, self.play_button, button);
        gui::set_text(self.gui, self.status_label, status);
        gui::set_value(self.gui, self.tempo_value, i32::from(self.tempo.get()));
        gui::set_value(self.gui, self.pitch_value, i32::from(self.pitch.get()));

        let (tempo_slider, tempo) = (self.tempo_slider, i32::from(self.tempo.get()));
        let (pitch_slider, pitch) = (self.pitch_slider, i32::from(self.pitch.get()));
        gui.present(surface, self.gui, |frame| {
            tempo_slider.draw(frame, tempo);
            pitch_slider.draw(frame, pitch);
        });
    }
}
