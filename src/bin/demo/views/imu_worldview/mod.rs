//! Stock IMU worldview application.
//!
//! This application owns its semantic/presentation state and concrete worldview
//! view. It consumes only the IMU capability plus shell-owned presentation
//! surfaces.

mod model;
mod view;

use embassy_time::Instant;

use hack_and_hike::{
    capabilities::{display::Surface, imu},
    ui::gui::GuiSurface,
};

pub(crate) use model::DisplayState;
use model::Model;
use view::View;

pub(crate) struct Application {
    model: Model,
    view: View,
}

impl Application {
    pub(crate) fn new(imu: imu::Imu) -> Self {
        Self {
            model: Model::new(imu),
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
