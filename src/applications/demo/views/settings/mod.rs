//! Stock display-settings screen.
//!
//! This screen owns brightness state, pointer interaction, and its concrete view.
//! It consumes only the display brightness control owned by the stock application.

mod model;
mod view;

use crate::{
    capabilities::display::{BrightnessControl, Surface},
    ui::gui::GuiSurface,
};

use super::super::navigation::ContentPointer;
use model::Model;
use view::View;

pub(crate) struct Application {
    model: Model,
    view: View,
}

impl Application {
    pub(crate) fn new(brightness: BrightnessControl) -> Self {
        Self {
            model: Model::new(brightness),
            view: View::new(),
        }
    }

    pub(crate) fn mark_dirty(&mut self) {
        self.model.mark_dirty();
    }

    pub(crate) fn handle_pointer(&mut self, pointer: ContentPointer) {
        if let Some(brightness) = self.view.handle_pointer(pointer) {
            self.model.set_brightness(brightness);
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
        self.view.present(gui_surface, surface, state.brightness);
        true
    }
}
