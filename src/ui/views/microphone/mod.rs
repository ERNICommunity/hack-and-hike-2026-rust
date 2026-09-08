//! Microphone view.
//!
//! KDL owns the static page structure and reserves the two waveform canvases.
//! The realtime waveform remains a view-specific direct renderer so audio-rate
//! updates never rebuild or repaint the GUI tree.

use embedded_gui::{font::FontId, prelude::*};

use crate::{data_plane, display::Display, waveform::WaveformFrame};

use super::super::gui::{GuiSurface, light_label_style};

mod waveform;

mod generated {
    use embedded_gui::prelude::*;
    embedded_gui::include_gui!("src/ui/views/microphone/microphone.kdl");
}

const NODE_CAPACITY: usize = 16;
const TEXT_CAPACITY: usize = 8;
const EVENT_CAPACITY: usize = 4;

type Context = GuiContext<'static, NODE_CAPACITY, TEXT_CAPACITY, EVENT_CAPACITY>;

#[derive(Clone, Copy, Debug)]
pub(super) struct Canvas {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
}

pub(crate) struct View {
    gui: &'static mut Context,
    left: Canvas,
    right: Canvas,
}

impl View {
    pub(crate) fn new() -> Self {
        // This context is only used for the static shell; the high-rate waveform
        // renderer bypasses embedded-gui entirely. PSRAM is therefore the right
        // home for the fixed-capacity widget tree/state.
        let gui = data_plane::leaked_value_with(|| Context::new(Rect::new(0, 0, 276, 240)));
        let app = generated::MicrophoneApp::build(gui)
            .expect("microphone KDL exceeds embedded-gui fixed capacities");

        // KDL remains the single source of layout geometry. The published
        // embedded-gui 0.2.x base label style is white-on-transparent, so light
        // firmware pages instantiate their visible labels with the shared light
        // style rather than duplicating coordinates in Rust.
        let left_label = gui
            .absolute_rect(app.widgets.left_label_slot)
            .expect("microphone left label layout missing");
        let right_label = gui
            .absolute_rect(app.widgets.right_label_slot)
            .expect("microphone right label layout missing");
        gui.add_label(
            left_label,
            "MIC L",
            light_label_style(FontId::Scaled6x10),
        )
        .expect("microphone left label exceeds embedded-gui fixed capacities");
        gui.add_label(
            right_label,
            "MIC R",
            light_label_style(FontId::Scaled6x10),
        )
        .expect("microphone right label exceeds embedded-gui fixed capacities");

        let left = canvas_from_rect(
            gui.absolute_rect(app.widgets.left_waveform)
                .expect("microphone left waveform layout missing"),
        );
        let right = canvas_from_rect(
            gui.absolute_rect(app.widgets.right_waveform)
                .expect("microphone right waveform layout missing"),
        );

        assert_canvas(left);
        assert_canvas(right);
        Self {
            gui,
            left,
            right,
        }
    }

    pub(crate) fn present_shell(&mut self, surface: &mut GuiSurface, display: &mut Display) {
        surface.present(display, self.gui);
    }

    pub(crate) fn render_waveform(&self, display: &mut Display, frame: &WaveformFrame) {
        waveform::render(display, self.left, self.right, frame);
    }
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
    assert_eq!(canvas.width % crate::waveform::POINTS, 0);
    assert!(crate::waveform::MAX_AMPLITUDE_PIXELS < canvas.height as i32 / 2);
}
