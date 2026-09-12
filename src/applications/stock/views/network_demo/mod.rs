//! Stock typed ESP-NOW messaging demonstration.
//!
//! The application owns its postcard schema, ping/pong behavior, presentation
//! state, and concrete view. The network capability remains schema-agnostic.

mod model;
mod view;

use embassy_time::Instant;
use serde::{Deserialize, Serialize};

use crate::{
    capabilities::{display::Surface, network},
    ui::gui::GuiSurface,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum DemoMessage {
    Ping { sequence: u32 },
    Pong { sequence: u32 },
}

pub(crate) use model::DisplayState;
use model::Model;
use view::View;

pub(crate) struct Application {
    model: Model,
    view: View,
}

impl Application {
    pub(crate) fn new(network: network::Network) -> Self {
        Self {
            model: Model::new(network),
            view: View::new(),
        }
    }

    pub(crate) fn mark_dirty(&mut self) {
        self.model.mark_dirty();
    }

    pub(crate) fn update_if_due(&mut self, now: Instant) {
        self.model.update_if_due(now);
    }

    pub(crate) fn present_shell(
        &mut self,
        gui_surface: &mut GuiSurface,
        surface: &mut Surface<'_>,
    ) {
        self.view.present_shell(gui_surface, surface);
    }

    pub(crate) fn present_if_dirty(
        &mut self,
        gui_surface: &mut GuiSurface,
        surface: &mut Surface<'_>,
    ) -> bool {
        let Some(state) = self.model.take_display() else {
            return false;
        };
        self.view.present(gui_surface, surface, &state);
        true
    }
}
