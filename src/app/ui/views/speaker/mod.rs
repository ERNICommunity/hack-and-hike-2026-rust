//! Interactive speaker-output view.
//!
//! KDL owns page geometry. `AppModel` owns persistent playback, tempo, and pitch
//! state while this view owns only transient pointer interaction and custom
//! touch-scale presentation inside the KDL control slots.

use core::fmt::Write as _;

use arrayvec::ArrayString;
use embedded_graphics::{
    prelude::*,
    primitives::{Circle, PrimitiveStyle},
};
use embedded_gui::prelude::*;

use crate::{
    app::model::SpeakerDisplay,
    capabilities::{
        audio::{PitchSemitones, TempoBpm},
        display::Surface,
    },
    support::memory::storage,
};

use super::super::{
    gui::{GuiFramebuffer, GuiSurface},
    navigation::{ContentPointer, PointerPhase},
};
use super::common;

mod generated {
    use embedded_gui::prelude::*;
    embedded_gui::include_gui!("src/app/ui/views/speaker/speaker.kdl");
}

const NODE_CAPACITY: usize = 16;
const TEXT_CAPACITY: usize = 8;
const EVENT_CAPACITY: usize = 8;
const SLIDER_HIT_MARGIN: i32 = 7;
const THUMB_DIAMETER: i32 = 20;
const THUMB_RADIUS: i32 = THUMB_DIAMETER / 2;
const TRACK_HEIGHT: u32 = 7;

type Context = GuiContext<'static, NODE_CAPACITY, TEXT_CAPACITY, EVENT_CAPACITY>;

#[derive(Clone, Copy)]
struct Geometry {
    title: Rect,
    status: Rect,
    play_button: Rect,
    chime_button: Rect,
    tempo_value: Rect,
    tempo_slider: Rect,
    pitch_value: Rect,
    pitch_slider: Rect,
    hint: Rect,
}

#[derive(Clone, Copy)]
enum Gesture {
    PlayButton,
    ChimeButton,
    Tempo,
    Pitch,
}

pub(crate) enum Action {
    TogglePlayback,
    PlayOneShot,
    SetTempo(TempoBpm),
    SetPitch(PitchSemitones),
}

pub(crate) struct View {
    gui: &'static mut Context,
    geometry: Geometry,
    gesture: Option<Gesture>,
}

impl View {
    pub(crate) fn new() -> Self {
        let gui = storage::leaked_value_with(|| Context::new(Rect::new(0, 0, 276, 240)));
        let app = generated::SpeakerApp::build(gui)
            .expect("speaker KDL exceeds embedded-gui fixed capacities");
        let geometry = Geometry {
            title: required_rect(gui, app.widgets.title_slot, "speaker title"),
            status: required_rect(gui, app.widgets.status_slot, "speaker status"),
            play_button: required_rect(gui, app.widgets.play_button_slot, "speaker play button"),
            chime_button: required_rect(gui, app.widgets.chime_button_slot, "speaker chime button"),
            tempo_value: required_rect(gui, app.widgets.tempo_value_slot, "speaker tempo value"),
            tempo_slider: required_rect(gui, app.widgets.tempo_slider_slot, "speaker tempo slider"),
            pitch_value: required_rect(gui, app.widgets.pitch_value_slot, "speaker pitch value"),
            pitch_slider: required_rect(gui, app.widgets.pitch_slider_slot, "speaker pitch slider"),
            hint: required_rect(gui, app.widgets.hint_slot, "speaker hint"),
        };

        Self {
            gui,
            geometry,
            gesture: None,
        }
    }

    pub(crate) fn present(
        &mut self,
        gui_surface: &mut GuiSurface,
        surface: &mut Surface<'_>,
        state: SpeakerDisplay,
    ) {
        let geometry = self.geometry;
        gui_surface.present_with_overlay(surface, self.gui, move |frame| {
            draw_speaker(frame, geometry, state);
        });
    }

    pub(crate) fn handle_pointer(&mut self, pointer: ContentPointer) -> Option<Action> {
        match pointer.phase {
            PointerPhase::Pressed if hits_slider(self.geometry.tempo_slider, pointer) => {
                self.gesture = Some(Gesture::Tempo);
                Some(Action::SetTempo(self.tempo_at(pointer.x)))
            }
            PointerPhase::Pressed if hits_slider(self.geometry.pitch_slider, pointer) => {
                self.gesture = Some(Gesture::Pitch);
                Some(Action::SetPitch(self.pitch_at(pointer.x)))
            }
            PointerPhase::Pressed if contains(self.geometry.play_button, pointer) => {
                self.gesture = Some(Gesture::PlayButton);
                None
            }
            PointerPhase::Pressed if contains(self.geometry.chime_button, pointer) => {
                self.gesture = Some(Gesture::ChimeButton);
                None
            }
            PointerPhase::Pressed => {
                self.gesture = None;
                None
            }
            PointerPhase::Moved if matches!(self.gesture, Some(Gesture::Tempo)) => {
                Some(Action::SetTempo(self.tempo_at(pointer.x)))
            }
            PointerPhase::Moved if matches!(self.gesture, Some(Gesture::Pitch)) => {
                Some(Action::SetPitch(self.pitch_at(pointer.x)))
            }
            PointerPhase::Released => {
                let gesture = self.gesture.take();
                match gesture {
                    Some(Gesture::Tempo) => Some(Action::SetTempo(self.tempo_at(pointer.x))),
                    Some(Gesture::Pitch) => Some(Action::SetPitch(self.pitch_at(pointer.x))),
                    Some(Gesture::PlayButton) if contains(self.geometry.play_button, pointer) => {
                        Some(Action::TogglePlayback)
                    }
                    Some(Gesture::ChimeButton) if contains(self.geometry.chime_button, pointer) => {
                        Some(Action::PlayOneShot)
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    fn tempo_at(&self, x: i32) -> TempoBpm {
        let value = slider_value_at(
            self.geometry.tempo_slider,
            x,
            i32::from(TempoBpm::MIN.get()),
            i32::from(TempoBpm::MAX.get()),
        ) as u16;
        TempoBpm::new(value).expect("tempo slider mapping must stay in range")
    }

    fn pitch_at(&self, x: i32) -> PitchSemitones {
        let value = slider_value_at(
            self.geometry.pitch_slider,
            x,
            i32::from(PitchSemitones::MIN.get()),
            i32::from(PitchSemitones::MAX.get()),
        ) as i8;
        PitchSemitones::new(value).expect("pitch slider mapping must stay in range")
    }
}

fn draw_speaker(frame: &mut GuiFramebuffer, geometry: Geometry, state: SpeakerDisplay) {
    common::draw_title(
        frame,
        "SPEAKER",
        geometry.title.x,
        geometry.title.y,
        common::dark_blue(),
    );
    common::draw_body(
        frame,
        if state.melody_playing {
            "MELODY PLAYING"
        } else {
            "MELODY STOPPED"
        },
        geometry.status.x,
        geometry.status.y,
        common::dark_gray(),
    );

    draw_button(
        frame,
        geometry.play_button,
        if state.melody_playing { "STOP" } else { "PLAY" },
        true,
    );
    draw_button(frame, geometry.chime_button, "PLAY CHIME", false);

    let mut text = ArrayString::<32>::new();
    let _ = write!(&mut text, "TEMPO  {} BPM", state.tempo.get());
    common::draw_body(
        frame,
        text.as_str(),
        geometry.tempo_value.x,
        geometry.tempo_value.y,
        common::black(),
    );
    draw_slider(
        frame,
        geometry.tempo_slider,
        i32::from(state.tempo.get()),
        i32::from(TempoBpm::MIN.get()),
        i32::from(TempoBpm::MAX.get()),
    );

    text.clear();
    let _ = write!(&mut text, "PITCH  {:+} semitones", state.pitch.get());
    common::draw_body(
        frame,
        text.as_str(),
        geometry.pitch_value.x,
        geometry.pitch_value.y,
        common::black(),
    );
    draw_slider(
        frame,
        geometry.pitch_slider,
        i32::from(state.pitch.get()),
        i32::from(PitchSemitones::MIN.get()),
        i32::from(PitchSemitones::MAX.get()),
    );

    common::draw_body(
        frame,
        "Sine melody + flash WAV",
        geometry.hint.x,
        geometry.hint.y,
        common::dark_gray(),
    );
}

fn draw_button(frame: &mut GuiFramebuffer, rect: Rect, label: &str, primary: bool) {
    let background = if primary {
        common::dark_blue()
    } else {
        common::light_gray()
    };
    let foreground = if primary {
        common::white()
    } else {
        common::black()
    };
    common::fill_rect(frame, rect, background);
    let label_rect = Rect::new(
        rect.x,
        rect.y + ((rect.h as i32 - common::BODY_LINE_HEIGHT) / 2).max(0),
        rect.w,
        common::BODY_LINE_HEIGHT as u32,
    );
    common::draw_centered_body(frame, label_rect, label, foreground);
}

fn draw_slider(frame: &mut GuiFramebuffer, rect: Rect, value: i32, min: i32, max: i32) {
    common::fill_rect(frame, rect, common::white());
    let (left, right) = slider_track_bounds(rect);
    let center_y = rect.y + rect.h as i32 / 2;
    let track_y = center_y - TRACK_HEIGHT as i32 / 2;
    common::fill_box(
        frame,
        left,
        track_y,
        (right - left + 1).max(1) as u32,
        TRACK_HEIGHT,
        common::light_gray(),
    );
    let range = (max - min).max(1);
    let offset = (value.clamp(min, max) - min) * (right - left).max(1);
    let thumb_x = left + (offset + range / 2) / range;
    common::fill_box(
        frame,
        left,
        track_y,
        (thumb_x - left + 1).max(1) as u32,
        TRACK_HEIGHT,
        common::dark_blue(),
    );
    let _ = Circle::new(
        Point::new(thumb_x - THUMB_RADIUS, center_y - THUMB_RADIUS),
        THUMB_DIAMETER as u32,
    )
    .into_styled(PrimitiveStyle::with_fill(common::dark_blue()))
    .draw(frame);
    let inner = 10i32;
    let _ = Circle::new(
        Point::new(thumb_x - inner / 2, center_y - inner / 2),
        inner as u32,
    )
    .into_styled(PrimitiveStyle::with_fill(common::white()))
    .draw(frame);
}

fn slider_value_at(rect: Rect, pointer_x: i32, min: i32, max: i32) -> i32 {
    let (left, right) = slider_track_bounds(rect);
    let span = (right - left).max(1);
    let x = pointer_x.clamp(left, right);
    min + (((x - left) * (max - min) + span / 2) / span)
}

fn slider_track_bounds(rect: Rect) -> (i32, i32) {
    let left = rect.x + THUMB_RADIUS;
    let right = rect.x + rect.w as i32 - THUMB_RADIUS - 1;
    (left, right.max(left + 1))
}

fn hits_slider(rect: Rect, pointer: ContentPointer) -> bool {
    pointer.x >= rect.x - SLIDER_HIT_MARGIN
        && pointer.x < rect.x + rect.w as i32 + SLIDER_HIT_MARGIN
        && pointer.y >= rect.y - SLIDER_HIT_MARGIN
        && pointer.y < rect.y + rect.h as i32 + SLIDER_HIT_MARGIN
}

fn contains(rect: Rect, pointer: ContentPointer) -> bool {
    pointer.x >= rect.x
        && pointer.x < rect.x + rect.w as i32
        && pointer.y >= rect.y
        && pointer.y < rect.y + rect.h as i32
}

fn required_rect(gui: &Context, id: WidgetId, name: &'static str) -> Rect {
    gui.absolute_rect(id)
        .unwrap_or_else(|| panic!("{name} layout missing"))
}
