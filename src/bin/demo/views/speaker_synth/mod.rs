//! Stock speaker-synth screen and behavior.
//!
//! This stock component owns melody/chime synthesis, tempo/pitch state, controls,
//! and its concrete view. It continuously feeds only the generic speaker PCM
//! capability owned by the stock application.

mod chime;
mod melody;
mod model;
mod view;

use hack_and_hike::{
    capabilities::{display::Surface, speaker::Speaker},
    ui::gui::GuiSurface,
};

use super::super::navigation::ContentPointer;

use model::Model;
pub(crate) use model::{PitchSemitones, SpeakerDisplay, TempoBpm};
use view::View;

#[derive(Clone, Copy, Debug)]
pub(crate) enum Action {
    TogglePlayback,
    PlayOneShot,
    SetTempo(TempoBpm),
    SetPitch(PitchSemitones),
}

pub(crate) struct Application {
    model: Model,
    view: View,
}

impl Application {
    pub(crate) fn new(speaker: Speaker) -> Self {
        Self {
            model: Model::new(speaker),
            view: View::new(),
        }
    }

    pub(crate) fn mark_dirty(&mut self) {
        self.model.mark_dirty();
    }

    pub(crate) fn update(&mut self) {
        self.model.update();
    }

    pub(crate) fn handle_pointer(&mut self, pointer: ContentPointer) {
        if let Some(action) = self.view.handle_pointer(pointer) {
            self.model.apply(action);
        }
    }

    pub(crate) fn present_if_dirty(
        &mut self,
        gui_surface: &mut GuiSurface,
        surface: &mut Surface<'_>,
    ) -> bool {
        let Some(state) = self.model.take_display() else {
            return false;
        };
        self.view.present(gui_surface, surface, state);
        true
    }
}
