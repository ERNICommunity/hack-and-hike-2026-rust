//! Microphone view.
//!
//! KDL owns the static page structure and reserves the label/waveform regions.
//! The labels are drawn with a native-resolution firmware font after the KDL
//! layout pass. The realtime waveform remains a view-specific direct renderer so
//! audio-rate updates never rebuild or repaint the GUI tree.

use embedded_gui::prelude::*;

use crate::{
    app::model::{MAX_AMPLITUDE_PIXELS, POINTS, WaveformFrame},
    capabilities::display::Display,
    support::memory::storage,
};

use super::super::gui::GuiSurface;
use super::common;

mod waveform;

mod generated {
    use embedded_gui::prelude::*;
    embedded_gui::include_gui!("src/app/ui/views/microphone/microphone.kdl");
}

const NODE_CAPACITY: usize = 12;
const TEXT_CAPACITY: usize = 4;
const EVENT_CAPACITY: usize = 4;

type Context = GuiContext<'static, NODE_CAPACITY, TEXT_CAPACITY, EVENT_CAPACITY>;

#[derive(Clone, Copy, Debug)]
struct Canvas {
    x: usize,
    y: usize,
    width: usize,
    height: usize,
}

pub(in crate::app::ui) struct View {
    gui: &'static mut Context,
    left_label: Rect,
    right_label: Rect,
    left: Canvas,
    right: Canvas,
}

impl View {
    pub(in crate::app::ui) fn new() -> Self {
        let gui = storage::leaked_value_with(|| Context::new(Rect::new(0, 0, 276, 240)));
        let app = generated::MicrophoneApp::build(gui)
            .expect("microphone KDL exceeds embedded-gui fixed capacities");

        let left_label = required_rect(gui, app.widgets.left_label_slot, "microphone left label");
        let right_label =
            required_rect(gui, app.widgets.right_label_slot, "microphone right label");
        let left = canvas_from_rect(required_rect(
            gui,
            app.widgets.left_waveform,
            "microphone left waveform",
        ));
        let right = canvas_from_rect(required_rect(
            gui,
            app.widgets.right_waveform,
            "microphone right waveform",
        ));

        assert_canvas(left);
        assert_canvas(right);
        Self {
            gui,
            left_label,
            right_label,
            left,
            right,
        }
    }

    pub(in crate::app::ui) fn present_shell(
        &mut self,
        surface: &mut GuiSurface,
        display: &mut Display,
    ) {
        let left_label = self.left_label;
        let right_label = self.right_label;
        surface.present_with_overlay(display, self.gui, move |frame| {
            common::draw_title(frame, "MIC L", left_label.x, left_label.y, common::black());
            common::draw_title(
                frame,
                "MIC R",
                right_label.x,
                right_label.y,
                common::black(),
            );
        });
    }

    pub(in crate::app::ui) fn render_waveform(&self, display: &mut Display, frame: &WaveformFrame) {
        waveform::render(display, self.left, self.right, frame);
    }
}

fn required_rect(gui: &Context, id: WidgetId, name: &'static str) -> Rect {
    gui.absolute_rect(id)
        .unwrap_or_else(|| panic!("{name} layout missing"))
}

fn canvas_from_rect(rect: Rect) -> Canvas {
    Canvas {
        x: rect.x.max(0) as usize,
        y: rect.y.max(0) as usize,
        width: rect.w as usize,
        height: rect.h as usize,
    }
}

fn assert_canvas(canvas: Canvas) {
    assert!(canvas.width > 0 && canvas.height > 0);
    assert!(canvas.x + canvas.width <= 276);
    assert!(canvas.y + canvas.height <= 240);
    assert_eq!(canvas.width % POINTS, 0);
    assert!(MAX_AMPLITUDE_PIXELS < canvas.height as i32 / 2);
}
