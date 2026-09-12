//! Stock speaker synth application.
//!
//! This application owns melody/chime synthesis, tempo/pitch state, controls,
//! and its concrete view. It continuously feeds only the generic speaker PCM
//! capability supplied by firmware composition.

mod chime;
mod melody;
mod model;
mod view;

use crate::{
    capabilities::{display::Surface, speaker::Speaker},
    ui::{
        gui::GuiSurface,
        navigation::ContentPointer,
    },
};

pub(crate) use model::{PitchSemitones, SpeakerDisplay, TempoBpm};
use model::Model;
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
