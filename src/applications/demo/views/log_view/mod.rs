//! Stock device-log application.
//!
//! Logging remains shared infrastructure. This application owns only the
//! history/presentation state, refresh policy, and concrete log view.

mod model;
mod view;

use embassy_time::Instant;

use crate::{capabilities::display::Surface, support::logging, ui::gui::GuiSurface};

use model::Model;
use view::View;

pub(crate) struct Application {
    model: Model,
    view: View,
}

impl Application {
    pub(crate) fn new(input: logging::Input) -> Self {
        let mut model = Model::new(input);
        model.refresh();
        Self {
            model,
            view: View::new(),
        }
    }

    pub(crate) fn mark_dirty(&mut self) {
        self.model.mark_dirty();
    }

    pub(crate) fn update_if_due(&mut self, now: Instant) {
        self.model.update_if_due(now);
    }

    pub(crate) fn present_if_dirty(
        &mut self,
        gui_surface: &mut GuiSurface,
        surface: &mut Surface<'_>,
    ) -> bool {
        let model = &mut self.model;
        let view = &mut self.view;
        model
            .with_text(|text| view.present(gui_surface, surface, text))
            .is_some()
    }

    pub(crate) fn present_current(
        &mut self,
        gui_surface: &mut GuiSurface,
        surface: &mut Surface<'_>,
    ) {
        if !self.present_if_dirty(gui_surface, surface) {
            self.view.present_shell(gui_surface, surface);
        }
    }
}
