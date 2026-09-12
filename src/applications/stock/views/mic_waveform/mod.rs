//! Stock microphone waveform application.
//!
//! The application owns waveform transformation, refresh policy, and its
//! concrete view. It consumes only the semantic microphone PCM capability plus
//! shell-owned presentation surfaces.

mod model;
mod view;

use embassy_time::Instant;

use crate::{
    capabilities::{display::Surface, mic},
    ui::gui::GuiSurface,
};

pub(crate) use model::{MAX_AMPLITUDE_PIXELS, POINTS, WaveformFrame};
use model::Model;
use view::View;

pub(crate) struct Application {
    model: Model,
    view: View,
}

impl Application {
    pub(crate) fn new(microphone: mic::Microphone) -> Self {
        Self {
            model: Model::new(microphone),
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

    pub(crate) fn render_if_dirty(&mut self, surface: &mut Surface<'_>) -> bool {
        let Some(frame) = self.model.take_frame() else {
            return false;
        };
        self.view.render_waveform(surface, &frame);
        true
    }
}
