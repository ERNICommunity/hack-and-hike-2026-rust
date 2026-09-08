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
    brightness: WidgetId,
}

impl View {
    pub(crate) fn new(brightness: BrightnessPercent) -> Self {
        let mut gui = Context::new(Rect::new(0, 0, 276, 240));
        let app = generated::SettingsApp::build(&mut gui)
            .expect("settings KDL exceeds embedded-gui fixed capacities");
        let brightness_rect = gui
            .absolute_rect(app.widgets.brightness_slot)
            .expect("settings brightness slot layout missing");
        let brightness = gui
            .add_themed_slider(
                brightness_rect,
                f32::from(brightness.get()),
                0.0,
                100.0,
            )
            .expect("settings brightness slider exceeds embedded-gui fixed capacities");
        while gui.pop_event().is_some() {}
        Self { gui, brightness }
    }

    pub(crate) fn present(&mut self, surface: &mut GuiSurface, display: &mut Display) {
        surface.present(display, &mut self.gui);
    }

    pub(crate) fn sync_brightness(&mut self, brightness: BrightnessPercent) {
        let expected = f32::from(brightness.get());
        if self.gui.slider_value(self.brightness) != Some(expected) {
            self.gui
                .set_slider_value(self.brightness, expected)
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
                if id == self.brightness {
                    // Brightness is a bounded, non-negative percentage. Adding
                    // half a step before the integer cast gives nearest-integer
                    // rounding without requiring a no_std float extension trait.
                    let value = self
                        .gui
                        .slider_value(self.brightness)
                        .unwrap_or(100.0)
                        .clamp(0.0, 100.0);
                    brightness = BrightnessPercent::new((value + 0.5) as u8);
                }
            }
        }
        brightness
    }
}
