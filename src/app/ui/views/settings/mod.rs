//! Interactive Settings view.
//!
//! KDL owns page geometry. `AppModel` owns the semantic brightness value while
//! this view owns only transient pointer interaction and the custom touch-scale
//! presentation inside the KDL slider slot.

use core::fmt::Write as _;

use arrayvec::ArrayString;
use embedded_graphics::{
    prelude::*,
    primitives::{Circle, PrimitiveStyle},
};
use embedded_gui::prelude::*;

use crate::{
    services::display::{BrightnessPercent, Display},
    support::memory::storage,
};

use super::super::{
    gui::{GuiFramebuffer, GuiSurface},
    navigation::{ContentPointer, PointerPhase},
};
use super::common;

mod generated {
    use embedded_gui::prelude::*;
    embedded_gui::include_gui!("src/app/ui/views/settings/settings.kdl");
}

const NODE_CAPACITY: usize = 16;
const TEXT_CAPACITY: usize = 8;
const EVENT_CAPACITY: usize = 8;
const SLIDER_HIT_MARGIN: i32 = 8;
const THUMB_DIAMETER: i32 = 22;
const THUMB_RADIUS: i32 = THUMB_DIAMETER / 2;
const TRACK_HEIGHT: u32 = 8;

type Context = GuiContext<'static, NODE_CAPACITY, TEXT_CAPACITY, EVENT_CAPACITY>;

#[derive(Clone, Copy)]
struct Geometry {
    title: Rect,
    value: Rect,
    slider: Rect,
    minimum: Rect,
    maximum: Rect,
    hint: Rect,
}

pub(super) struct View {
    gui: &'static mut Context,
    geometry: Geometry,
    dragging_brightness: bool,
}

impl View {
    pub(super) fn new() -> Self {
        let gui = storage::leaked_value_with(|| Context::new(Rect::new(0, 0, 276, 240)));
        let app = generated::SettingsApp::build(gui)
            .expect("settings KDL exceeds embedded-gui fixed capacities");
        let geometry = Geometry {
            title: required_rect(gui, app.widgets.title_slot, "settings title"),
            value: required_rect(
                gui,
                app.widgets.brightness_value_slot,
                "settings brightness value",
            ),
            slider: required_rect(
                gui,
                app.widgets.brightness_slot,
                "settings brightness slider",
            ),
            minimum: required_rect(gui, app.widgets.minimum_slot, "settings minimum"),
            maximum: required_rect(gui, app.widgets.maximum_slot, "settings maximum"),
            hint: required_rect(gui, app.widgets.hint_slot, "settings hint"),
        };

        Self {
            gui,
            geometry,
            dragging_brightness: false,
        }
    }

    pub(super) fn present(
        &mut self,
        surface: &mut GuiSurface,
        display: &mut Display,
        brightness: BrightnessPercent,
    ) {
        let geometry = self.geometry;
        surface.present_with_overlay(display, self.gui, move |frame| {
            draw_settings(frame, geometry, brightness);
        });
    }

    /// Handle one content-space pointer event and return the newest semantic
    /// brightness action. Persistent brightness state remains in `AppModel`.
    pub(super) fn handle_pointer(&mut self, pointer: ContentPointer) -> Option<BrightnessPercent> {
        match pointer.phase {
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
        }
    }

    fn pointer_hits_brightness(&self, pointer: ContentPointer) -> bool {
        let rect = self.geometry.slider;
        let left = rect.x - SLIDER_HIT_MARGIN;
        let top = rect.y - SLIDER_HIT_MARGIN;
        let right = rect.x + rect.w as i32 + SLIDER_HIT_MARGIN;
        let bottom = rect.y + rect.h as i32 + SLIDER_HIT_MARGIN;
        pointer.x >= left && pointer.x < right && pointer.y >= top && pointer.y < bottom
    }

    fn brightness_at(&self, pointer_x: i32) -> BrightnessPercent {
        let (left, right) = slider_track_bounds(self.geometry.slider);
        let span = (right - left).max(1);
        let x = pointer_x.clamp(left, right);
        let range = i32::from(BrightnessPercent::FULL.get() - BrightnessPercent::MIN.get());
        let offset = ((x - left) * range + span / 2) / span;
        let percent = BrightnessPercent::MIN.get() + offset as u8;
        BrightnessPercent::new(percent)
            .expect("slider mapping must produce a visible brightness percentage")
    }
}

fn draw_settings(frame: &mut GuiFramebuffer, geometry: Geometry, brightness: BrightnessPercent) {
    common::draw_title(
        frame,
        "SETTINGS",
        geometry.title.x,
        geometry.title.y,
        common::dark_blue(),
    );

    let mut value = ArrayString::<32>::new();
    let _ = write!(&mut value, "DISPLAY BRIGHTNESS  {}%", brightness.get());
    common::draw_body(
        frame,
        value.as_str(),
        geometry.value.x,
        geometry.value.y,
        common::black(),
    );

    draw_brightness_slider(frame, geometry.slider, brightness);
    common::draw_centered_body(frame, geometry.minimum, "DIM", common::dark_gray());
    common::draw_centered_body(frame, geometry.maximum, "MAX", common::dark_gray());
    common::draw_body(
        frame,
        "Tap or drag to adjust",
        geometry.hint.x,
        geometry.hint.y,
        common::dark_gray(),
    );
}

fn draw_brightness_slider(frame: &mut GuiFramebuffer, rect: Rect, brightness: BrightnessPercent) {
    common::fill_rect(frame, rect, common::white());

    let (left, right) = slider_track_bounds(rect);
    let center_y = rect.y + rect.h as i32 / 2;
    let track_y = center_y - TRACK_HEIGHT as i32 / 2;
    let track_width = (right - left + 1).max(1) as u32;
    common::fill_box(
        frame,
        left,
        track_y,
        track_width,
        TRACK_HEIGHT,
        common::light_gray(),
    );

    let range = i32::from(BrightnessPercent::FULL.get() - BrightnessPercent::MIN.get());
    let offset = i32::from(brightness.get() - BrightnessPercent::MIN.get());
    let span = (right - left).max(1);
    let thumb_x = left + (offset * span + range / 2) / range.max(1);
    let active_width = (thumb_x - left + 1).max(1) as u32;
    common::fill_box(
        frame,
        left,
        track_y,
        active_width,
        TRACK_HEIGHT,
        common::dark_blue(),
    );

    let outer = Circle::new(
        Point::new(thumb_x - THUMB_RADIUS, center_y - THUMB_RADIUS),
        THUMB_DIAMETER as u32,
    );
    let _ = outer
        .into_styled(PrimitiveStyle::with_fill(common::dark_blue()))
        .draw(frame);
    let inner_diameter = 12i32;
    let inner = Circle::new(
        Point::new(thumb_x - inner_diameter / 2, center_y - inner_diameter / 2),
        inner_diameter as u32,
    );
    let _ = inner
        .into_styled(PrimitiveStyle::with_fill(common::white()))
        .draw(frame);
}

fn slider_track_bounds(rect: Rect) -> (i32, i32) {
    let left = rect.x + THUMB_RADIUS;
    let right = rect.x + rect.w as i32 - THUMB_RADIUS - 1;
    (left, right.max(left + 1))
}

fn required_rect(gui: &Context, id: WidgetId, name: &'static str) -> Rect {
    gui.absolute_rect(id)
        .unwrap_or_else(|| panic!("{name} layout missing"))
}
