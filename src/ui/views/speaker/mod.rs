//! Speaker output view.

use embedded_gui::prelude::*;

use crate::{data_plane, display::Display};

use super::super::gui::GuiSurface;
use super::common;

mod generated {
    use embedded_gui::prelude::*;
    embedded_gui::include_gui!("src/ui/views/speaker/speaker.kdl");
}

const NODE_CAPACITY: usize = 6;
const TEXT_CAPACITY: usize = 4;
const EVENT_CAPACITY: usize = 2;

type Context = GuiContext<'static, NODE_CAPACITY, TEXT_CAPACITY, EVENT_CAPACITY>;

#[derive(Clone, Copy)]
struct Geometry {
    title: Rect,
    subtitle: Rect,
}

pub(crate) struct View {
    gui: &'static mut Context,
    geometry: Geometry,
}

impl View {
    pub(crate) fn new() -> Self {
        let gui = data_plane::leaked_value_with(|| Context::new(Rect::new(0, 0, 276, 240)));
        let app = generated::SpeakerApp::build(gui)
            .expect("speaker KDL exceeds embedded-gui fixed capacities");
        Self {
            geometry: Geometry {
                title: required_rect(gui, app.widgets.title_slot, "speaker title"),
                subtitle: required_rect(gui, app.widgets.subtitle_slot, "speaker subtitle"),
            },
            gui,
        }
    }

    pub(crate) fn present(&mut self, surface: &mut GuiSurface, display: &mut Display) {
        let geometry = self.geometry;
        surface.present_with_overlay(display, self.gui, move |frame| {
            common::draw_centered_title(frame, geometry.title, "SPEAKER", common::dark_blue());
            common::draw_centered_body(
                frame,
                geometry.subtitle,
                "Speaker output",
                common::dark_gray(),
            );
        });
    }
}

fn required_rect(gui: &Context, id: WidgetId, name: &'static str) -> Rect {
    gui.absolute_rect(id).unwrap_or_else(|| panic!("{name} layout missing"))
}
