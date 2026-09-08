//! Interactive Settings view.
//!
//! KDL owns layout and widget construction. This module owns the semantic bridge:
//! content pointer input becomes a brightness percentage action, while the
//! application model remains the authoritative brightness state.

use embedded_gui::prelude::*;

use crate::{display::Display, display_control::BrightnessPercent};

use super::super::{gui::GuiSurface, navigation::{ContentPointer, PointerPhase}};

mod generated {
    use embedded_gui::prelude::*;
    embedded_gui::include_gui!("src/ui/views/settings/settings.kdl");
}

const NODE_CAPACITY: usize = 16;
const TEXT_CAPACITY: usize = 16;
const EVENT_CAPACITY: usize = 8;

type Context = GuiContext<'static, NODE_CAPACITY, TEXT_CAPACITY, EVENT_CAPACITY>;

pub(crate) struct View {
    gui: Context,
    app: generated::SettingsApp,
}

impl View {
    pub(crate) fn new(brightness: BrightnessPercent) -> Self {
        let mut gui = Context::new(Rect::new(0, 0, 276, 240));
        let app = generated::SettingsApp::build(&mut gui)
            .expect("settings KDL exceeds embedded-gui fixed capacities");
        gui.set_slider_value(app.widgets.brightness, f32::from(brightness.get()))
            .expect("settings brightness widget is not a slider");
        while gui.pop_event().is_some() {}
        Self { gui, app }
    }

    pub(crate) fn present(&mut self, surface: &mut GuiSurface, display: &mut Display) {
        surface.present(display, &mut self.gui);
    }

    pub(crate) fn sync_brightness(&mut self, brightness: BrightnessPercent) {
        let expected = f32::from(brightness.get());
        if self.gui.slider_value(self.app.widgets.brightness) != Some(expected) {
            self.gui
                .set_slider_value(self.app.widgets.brightness, expected)
                .expect("settings brightness widget is not a slider");
            while self.gui.pop_event().is_some() {}
        }
    }

    /// Handle one content-space pointer event and return the newest semantic
    /// brightness action produced by the slider, if any.
    pub(crate) fn handle_pointer(&mut self, pointer: ContentPointer) -> Option<BrightnessPercent> {
        let state = match pointer.phase {
            PointerPhase::Pressed => PointerState::Pressed,
            PointerPhase::Moved => PointerState::Moved,
            PointerPhase::Released => PointerState::Released,
        };
        self.gui
            .handle_input(InputEvent::Pointer {
                x: pointer.x,
                y: pointer.y,
                state,
                button: PointerButton::Primary,
            })
            .expect("settings input event capacity exceeded");

        let mut brightness = None;
        while let Some(event) = self.gui.pop_event() {
            if let UiEvent::ValueChanged(id) = event {
                if id == self.app.widgets.brightness {
                    let value = self
                        .gui
                        .slider_value(self.app.widgets.brightness)
                        .unwrap_or(100.0)
                        .round()
                        .clamp(0.0, 100.0) as u8;
                    brightness = BrightnessPercent::new(value);
                }
            }
        }
        brightness
    }
}
