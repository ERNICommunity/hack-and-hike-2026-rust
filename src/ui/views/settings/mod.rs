//! Interactive Settings view.
//!
//! KDL owns page geometry. This module owns the semantic bridge from touch to a
//! bounded brightness percentage while `AppModel` remains authoritative state.
//! View-specific compatibility code is intentionally kept here: embedded-gui
//! 0.2.5 renders the slider correctly but does not update its scalar value from
//! pointer drags on this input path.

use embedded_gui::{font::FontId, prelude::*};

use crate::{display::Display, display_control::BrightnessPercent};

use super::super::{
    gui::{GuiSurface, light_label_style},
    navigation::{ContentPointer, PointerPhase},
};

mod generated {
    use embedded_gui::prelude::*;
    embedded_gui::include_gui!("src/ui/views/settings/settings.kdl");
}

const NODE_CAPACITY: usize = 20;
const TEXT_CAPACITY: usize = 16;
const EVENT_CAPACITY: usize = 8;
const SLIDER_HIT_MARGIN: i32 = 8;

type Context = GuiContext<'static, NODE_CAPACITY, TEXT_CAPACITY, EVENT_CAPACITY>;

pub(crate) struct View {
    gui: Context,
    brightness: WidgetId,
    brightness_value: WidgetId,
    brightness_rect: Rect,
    dragging_brightness: bool,
}

impl View {
    pub(crate) fn new(brightness: BrightnessPercent) -> Self {
        let mut gui = Context::new(Rect::new(0, 0, 276, 240));
        let app = generated::SettingsApp::build(&mut gui)
            .expect("settings KDL exceeds embedded-gui fixed capacities");

        // The KDL slots are the single source of geometry. Visible labels are
        // instantiated with an explicit light-page style because embedded-gui's
        // base label style is white-on-transparent.
        let title_rect = required_rect(&gui, app.widgets.title_slot, "settings title");
        let value_rect = required_rect(
            &gui,
            app.widgets.brightness_value_slot,
            "settings brightness value",
        );
        let brightness_rect = required_rect(
            &gui,
            app.widgets.brightness_slot,
            "settings brightness slider",
        );
        let minimum_rect = required_rect(&gui, app.widgets.minimum_slot, "settings minimum");
        let maximum_rect = required_rect(&gui, app.widgets.maximum_slot, "settings maximum");
        let hint_rect = required_rect(&gui, app.widgets.hint_slot, "settings hint");

        gui.add_label(
            title_rect,
            "SETTINGS",
            light_label_style(FontId::Scaled6x10),
        )
        .expect("settings title exceeds embedded-gui fixed capacities");
        let brightness_value = gui
            .add_value_label(
                value_rect,
                "DISPLAY BRIGHTNESS %",
                i32::from(brightness.get()),
                light_label_style(FontId::Medium4x7),
            )
            .expect("settings value label exceeds embedded-gui fixed capacities");
        let brightness_widget = gui
            .add_themed_slider(
                brightness_rect,
                f32::from(brightness.get()),
                0.0,
                100.0,
            )
            .expect("settings brightness slider exceeds embedded-gui fixed capacities");
        gui.add_label(
            minimum_rect,
            "0%",
            light_label_style(FontId::Medium4x7),
        )
        .expect("settings minimum label exceeds embedded-gui fixed capacities");
        gui.add_label(
            maximum_rect,
            "100%",
            light_label_style(FontId::Medium4x7),
        )
        .expect("settings maximum label exceeds embedded-gui fixed capacities");
        gui.add_label(
            hint_rect,
            "Tap or drag to adjust backlight",
            light_label_style(FontId::Medium4x7),
        )
        .expect("settings hint exceeds embedded-gui fixed capacities");

        drain_events(&mut gui);
        Self {
            gui,
            brightness: brightness_widget,
            brightness_value,
            brightness_rect,
            dragging_brightness: false,
        }
    }

    pub(crate) fn present(&mut self, surface: &mut GuiSurface, display: &mut Display) {
        surface.present(display, &mut self.gui);
    }

    pub(crate) fn sync_brightness(&mut self, brightness: BrightnessPercent) {
        self.set_local_brightness(brightness);
    }

    /// Handle one content-space pointer event and return the newest semantic
    /// brightness action produced by the slider, if any.
    pub(crate) fn handle_pointer(&mut self, pointer: ContentPointer) -> Option<BrightnessPercent> {
        let state = match pointer.phase {
            PointerPhase::Pressed => PointerState::Pressed,
            PointerPhase::Moved => PointerState::Moved,
            PointerPhase::Released => PointerState::Released,
        };

        // Keep embedded-gui's normal focus/pressed semantics active even though
        // 0.2.5 needs the view-specific scalar mapping below for touch sliders.
        self.gui
            .handle_input(InputEvent::Pointer {
                x: pointer.x,
                y: pointer.y,
                state,
                button: PointerButton::Primary,
            })
            .expect("settings input event capacity exceeded");

        let brightness = match pointer.phase {
            PointerPhase::Pressed if self.pointer_hits_brightness(pointer) => {
                self.dragging_brightness = true;
                Some(self.brightness_at(pointer.x))
            }
            PointerPhase::Moved if self.dragging_brightness => Some(self.brightness_at(pointer.x)),
            PointerPhase::Released if self.dragging_brightness => {
                self.dragging_brightness = false;
                Some(self.brightness_at(pointer.x))
            }
            PointerPhase::Released => {
                self.dragging_brightness = false;
                None
            }
            _ => None,
        };

        // The semantic value above is authoritative for this adapter. Discard
        // framework-local events so stale ValueChanged notifications cannot be
        // observed on a later gesture.
        drain_events(&mut self.gui);
        brightness
    }

    fn pointer_hits_brightness(&self, pointer: ContentPointer) -> bool {
        let rect = self.brightness_rect;
        let left = rect.x - SLIDER_HIT_MARGIN;
        let top = rect.y - SLIDER_HIT_MARGIN;
        let right = rect.x + rect.w as i32 + SLIDER_HIT_MARGIN;
        let bottom = rect.y + rect.h as i32 + SLIDER_HIT_MARGIN;
        pointer.x >= left && pointer.x < right && pointer.y >= top && pointer.y < bottom
    }

    fn brightness_at(&mut self, pointer_x: i32) -> BrightnessPercent {
        let left = self.brightness_rect.x;
        let right = left + self.brightness_rect.w.saturating_sub(1) as i32;
        let span = (right - left).max(1);
        let x = pointer_x.clamp(left, right);
        let percent = (((x - left) * 100 + span / 2) / span) as u8;
        let brightness = BrightnessPercent::new(percent)
            .expect("slider mapping must produce a valid brightness percentage");
        self.set_local_brightness(brightness);
        brightness
    }

    fn set_local_brightness(&mut self, brightness: BrightnessPercent) {
        let slider_value = f32::from(brightness.get());
        if self.gui.slider_value(self.brightness) != Some(slider_value) {
            self.gui
                .set_slider_value(self.brightness, slider_value)
                .expect("settings brightness widget is not a slider");
        }
        self.gui
            .set_value_label(self.brightness_value, i32::from(brightness.get()))
            .expect("settings brightness value widget is not a value label");
        drain_events(&mut self.gui);
    }
}

fn required_rect(gui: &Context, id: WidgetId, name: &'static str) -> Rect {
    gui.absolute_rect(id).unwrap_or_else(|| panic!("{name} layout missing"))
}

fn drain_events(gui: &mut Context) {
    while gui.pop_event().is_some() {}
}
