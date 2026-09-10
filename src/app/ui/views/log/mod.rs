//! Device log view.
//!
//! KDL owns the title/body geometry. The scrolling log remains a specialized
//! dense-text component and uses a native 6x12 font rather than a scaled glyph.

use embedded_gui::prelude::*;

use crate::{data_plane, display::Display};

use super::super::gui::GuiSurface;
use super::common;

mod generated {
    use embedded_gui::prelude::*;
    embedded_gui::include_gui!("src/app/ui/views/log/log.kdl");
}

const NODE_CAPACITY: usize = 6;
const TEXT_CAPACITY: usize = 4;
const EVENT_CAPACITY: usize = 2;

type Context = GuiContext<'static, NODE_CAPACITY, TEXT_CAPACITY, EVENT_CAPACITY>;

#[derive(Clone, Copy)]
struct Geometry {
    title: Rect,
    body: Rect,
}

pub(crate) struct View {
    gui: &'static mut Context,
    geometry: Geometry,
}

impl View {
    pub(crate) fn new() -> Self {
        let gui = data_plane::leaked_value_with(|| Context::new(Rect::new(0, 0, 276, 240)));
        let app = generated::LogApp::build(gui).expect("log KDL exceeds embedded-gui capacities");
        Self {
            geometry: Geometry {
                title: required_rect(gui, app.widgets.title_slot, "log title"),
                body: required_rect(gui, app.widgets.body_slot, "log body"),
            },
            gui,
        }
    }

    pub(crate) fn present_shell(&mut self, surface: &mut GuiSurface, display: &mut Display) {
        let geometry = self.geometry;
        surface.present_with_overlay(display, self.gui, move |frame| {
            draw_title(frame, geometry);
        });
    }

    pub(crate) fn present(&mut self, surface: &mut GuiSurface, display: &mut Display, text: &str) {
        let geometry = self.geometry;
        surface.present_with_overlay(display, self.gui, move |frame| {
            draw_title(frame, geometry);
            let line_count = (geometry.body.h as i32 / common::DENSE_LINE_HEIGHT).max(1) as usize;
            let visible = trailing_lines(text, line_count);
            for (index, line) in visible.lines().take(line_count).enumerate() {
                common::draw_dense(
                    frame,
                    line,
                    geometry.body.x,
                    geometry.body.y + index as i32 * common::DENSE_LINE_HEIGHT,
                    common::black(),
                );
            }
        });
    }
}

fn draw_title(frame: &mut super::super::gui::GuiFramebuffer, geometry: Geometry) {
    common::draw_title(
        frame,
        "LOG",
        geometry.title.x,
        geometry.title.y,
        common::dark_blue(),
    );
}

fn trailing_lines(text: &str, line_count: usize) -> &str {
    if line_count == 0 || text.is_empty() {
        return "";
    }

    let bytes = text.as_bytes();
    let mut seen = 0usize;
    for index in (0..bytes.len()).rev() {
        if bytes[index] != b'\n' {
            continue;
        }
        seen += 1;
        if seen > line_count {
            return &text[index + 1..];
        }
    }
    text
}

fn required_rect(gui: &Context, id: WidgetId, name: &'static str) -> Rect {
    gui.absolute_rect(id).unwrap_or_else(|| panic!("{name} layout missing"))
}
