//! A looping melody and a chime, with tempo and pitch sliders.
//!
//! Audio keeps playing while another screen is visible, so the speaker queue
//! is fed from `update`, which the shell calls for every screen. The melody
//! and the chime are mixed: both can play at once.

mod chime;
mod melody;

use embedded_gui::WidgetId;
use hack_and_hike::{
    capabilities::{
        audio::{self, Speaker},
        display::Surface,
        touch::TouchEvent,
    },
    ui::{Canvas, gui, theme, widgets::Slider},
};

use crate::{layout, screens::Screen, styles};

use chime::FlashChime;
use melody::MelodySynth;

// The layout file becomes Rust at compile time: a `...App` struct with a
// `build` function and one `WidgetId` per named node.
mod generated {
    use embedded_gui::prelude::*;
    embedded_gui::include_gui!("src/bin/demo/screens/speaker/speaker.kdl");
}

/// Room for widgets in this screen's GUI context: the KDL nodes plus the
/// widgets added in code.
const NODES: usize = 16;
const _: () = assert!(generated::SpeakerApp::WIDTH == layout::CONTENT_SIZE.width);
const _: () = assert!(generated::SpeakerApp::HEIGHT == layout::CONTENT_SIZE.height);
/// Frames generated per chunk when topping up the speaker queue.
const PCM_CHUNK_FRAMES: usize = 128;
/// The same chunk in samples.
const PCM_CHUNK_SAMPLES: usize = PCM_CHUNK_FRAMES * audio::CHANNELS;

/// A slider value outside the allowed range.
#[derive(Clone, Copy, Debug)]
pub(crate) struct OutOfRange;

/// Melody tempo in quarter-note beats per minute, 60 to 180.
///
/// A newtype: a plain `u16` could be a tempo, a pitch or anything else; a
/// `TempoBpm` can only be a valid tempo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TempoBpm(u16);

impl TempoBpm {
    /// The slowest tempo.
    pub(crate) const MIN: Self = Self(60);
    /// The tempo at boot.
    pub(crate) const DEFAULT: Self = Self(120);
    /// The fastest tempo.
    pub(crate) const MAX: Self = Self(180);

    /// Beats per minute.
    pub(crate) const fn get(self) -> u16 {
        self.0
    }
}

/// For slider values.
impl TryFrom<i32> for TempoBpm {
    type Error = OutOfRange;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        let value = u16::try_from(value).map_err(|_| OutOfRange)?;
        if (Self::MIN.0..=Self::MAX.0).contains(&value) {
            Ok(Self(value))
        } else {
            Err(OutOfRange)
        }
    }
}

/// Transposition of the melody in semitones, -12 to +12 (an octave down or
/// up).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PitchSemitones(i8);

impl PitchSemitones {
    /// One octave down.
    pub(crate) const MIN: Self = Self(-12);
    /// No transposition: the pitch at boot.
    pub(crate) const CENTER: Self = Self(0);
    /// One octave up.
    pub(crate) const MAX: Self = Self(12);

    /// Semitones up (positive) or down (negative).
    pub(crate) const fn get(self) -> i8 {
        self.0
    }
}

/// For slider values.
impl TryFrom<i32> for PitchSemitones {
    type Error = OutOfRange;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        let value = i8::try_from(value).map_err(|_| OutOfRange)?;
        if (Self::MIN.0..=Self::MAX.0).contains(&value) {
            Ok(Self(value))
        } else {
            Err(OutOfRange)
        }
    }
}

/// The speaker screen, its two sound sources and its controls.
pub(crate) struct SpeakerScreen {
    speaker: Speaker,
    melody: MelodySynth,
    chime: FlashChime,
    /// Whether the melody is on.
    playing: bool,
    tempo: TempoBpm,
    pitch: PitchSemitones,
    gui: &'static mut gui::Context<NODES>,
    /// PLAY / STOP.
    play_button: WidgetId,
    /// PLAY CHIME.
    chime_button: WidgetId,
    /// "MELODY PLAYING" / "MELODY STOPPED".
    status_label: WidgetId,
    /// The tempo readout.
    tempo_value: WidgetId,
    /// The pitch readout.
    pitch_value: WidgetId,
    tempo_slider: Slider,
    pitch_slider: Slider,
    /// Whether the screen needs a redraw.
    dirty: bool,
}

impl SpeakerScreen {
    /// Build the layout, the readouts and the sliders; nothing plays yet.
    pub(crate) fn new(speaker: Speaker) -> Self {
        let gui = gui::context::<NODES>(layout::CONTENT_SIZE.width, layout::CONTENT_SIZE.height);
        let app = generated::SpeakerApp::build(gui).expect("speaker.kdl fits the GUI capacities");
        let tempo_value = gui::add_value_label(
            gui,
            app.widgets.tempo_value,
            "TEMPO BPM",
            i32::from(TempoBpm::DEFAULT.get()),
            styles::value(),
        );
        let pitch_value = gui::add_value_label(
            gui,
            app.widgets.pitch_value,
            "PITCH SEMITONES",
            i32::from(PitchSemitones::CENTER.get()),
            styles::value(),
        );

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

    /// The next audio frame: the melody (when on) plus the chime, clamped to
    /// the 16-bit range.
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

    fn handle_touch(&mut self, event: TouchEvent) {
        let mut clicked = None;
        gui::click_buttons(self.gui, event, |id| clicked = Some(id));
        if clicked == Some(self.play_button) {
            self.playing = !self.playing;
            if self.playing {
                self.melody.restart();
            }
        } else if clicked == Some(self.chime_button) {
            self.chime.restart();
        }

        if let Some(Ok(tempo)) = self
            .tempo_slider
            .handle_touch(event)
            .map(TempoBpm::try_from)
        {
            self.tempo = tempo;
        }
        if let Some(Ok(pitch)) = self
            .pitch_slider
            .handle_touch(event)
            .map(PitchSemitones::try_from)
        {
            self.pitch = pitch;
        }
        // Buttons change appearance while pressed, so redraw on every touch.
        self.dirty = true;
    }

    fn present(&mut self, canvas: &mut Canvas, surface: &mut Surface<'_>) {
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

        canvas.clear(theme::WHITE);
        gui::render(self.gui, canvas);
        self.tempo_slider.draw(canvas, i32::from(self.tempo.get()));
        self.pitch_slider.draw(canvas, i32::from(self.pitch.get()));
        canvas.show(surface);
    }
}
