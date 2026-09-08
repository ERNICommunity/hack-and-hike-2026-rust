//! Microphone view.
//!
//! KDL owns the static page structure and reserves the two waveform canvases.
//! The realtime waveform remains a view-specific direct renderer so audio-rate
//! updates never rebuild or repaint the GUI tree.

use embedded_gui::prelude::*;

use crate::{display::Display, waveform::WaveformFrame};

use super::super::gui::GuiSurface;

mod waveform;

mod generated {
    use embedded_gui::prelude::*;
    embedded_gui::include_gui!("src/ui/views/microphone/microphone.kdl");
}

const NODE_CAPACITY: usize = 12;
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
    gui: Context,
    left: Canvas,
    right: Canvas,
}

impl View {
    pub(crate) fn new() -> Self {
        let mut gui = Context::new(Rect::new(0, 0, 276, 240));
        let app = generated::MicrophoneApp::build(&mut gui)
            .expect("microphone KDL exceeds embedded-gui fixed capacities");
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
        surface.present(display, &mut self.gui);
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
