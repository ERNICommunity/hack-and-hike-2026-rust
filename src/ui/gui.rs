//! Glue between `embedded-gui` and the rest of the firmware.
//!
//! A graphical screen describes its layout in a KDL file, builds it into a
//! [`Context`] once, renders the widgets into a [`Canvas`] with [`render`]
//! and forwards touches with [`click_buttons`].

use embedded_graphics::{
    prelude::{Point, Size},
    primitives::Rectangle,
};
use embedded_gui::{
    GuiContext, InputEvent, PointerButton, PointerState, PropertyKey, PropertyValue, Rect, Style,
    UiEvent, WidgetId,
};
use log::debug;

use crate::{capabilities::touch::TouchEvent, support::memory::storage};

use super::Canvas;

/// UI events one touch may queue before the screen drains them; a single tap
/// produces about nine.
pub const EVENTS: usize = 16;
pub const DIRTY_RECTS: usize = 8;

/// The `embedded-gui` context of one screen with room for `NODES` widgets.
pub type Context<const NODES: usize> = GuiContext<'static, NODES, EVENTS, DIRTY_RECTS>;

/// Allocate a screen's GUI context in PSRAM; it lives for the rest of the run.
pub fn context<const NODES: usize>(width: u32, height: u32) -> &'static mut Context<NODES> {
    storage::leaked_value(|| Context::new(Rect::new(0, 0, width, height)))
}

/// The rectangle a KDL node occupies.
///
/// Panics when the id is unknown: the layout is fixed at compile time, so a
/// missing node is a programming error, not a runtime condition.
pub fn slot<const NODES: usize>(gui: &Context<NODES>, id: WidgetId) -> Rectangle {
    let rect = gui_rect(gui, id);
    Rectangle::new(Point::new(rect.x, rect.y), Size::new(rect.w, rect.h))
}

fn gui_rect<const NODES: usize>(gui: &Context<NODES>, id: WidgetId) -> Rect {
    gui.absolute_rect(id)
        .expect("every KDL node has a rectangle after build()")
}

/// Put a numeric readout (a caption on the left, a number on the right) into
/// the slot of the KDL node `id`. Change the number with [`set_value`].
pub fn add_value_label<const NODES: usize>(
    gui: &mut Context<NODES>,
    id: WidgetId,
    caption: &'static str,
    value: i32,
    style: Style,
) -> WidgetId {
    let rect = gui_rect(gui, id);
    gui.add_value_label(rect, caption, value, style)
        .expect("the GUI context has room for a value label")
}

/// Change the text of a label or button.
pub fn set_text<const NODES: usize>(gui: &mut Context<NODES>, id: WidgetId, text: &'static str) {
    if let Err(error) = gui.set_widget_property(id, PropertyKey::Text, PropertyValue::Str(text)) {
        debug!("GUI text update failed: {:?}", error);
    }
}

/// Change the number shown by a `value_label`.
pub fn set_value<const NODES: usize>(gui: &mut Context<NODES>, id: WidgetId, value: i32) {
    if let Err(error) = gui.set_value_label(id, value) {
        debug!("GUI value update failed: {:?}", error);
    }
}

/// Deliver a touch to the widgets and call `on_click` for every button it
/// clicked. `event` must be in the coordinates of the GUI context.
pub fn click_buttons<const NODES: usize>(
    gui: &mut Context<NODES>,
    event: TouchEvent,
    mut on_click: impl FnMut(WidgetId),
) {
    let (point, state) = match event {
        TouchEvent::Pressed(point) => (point, PointerState::Pressed),
        TouchEvent::Moved(point) => (point, PointerState::Moved),
        TouchEvent::Released(point) => (point, PointerState::Released),
    };
    let input = InputEvent::Pointer {
        x: point.x,
        y: point.y,
        state,
        button: PointerButton::Primary,
    };
    if let Err(error) = gui.handle_input(input) {
        debug!("GUI input failed: {:?}", error);
    }
    while let Some(event) = gui.pop_event() {
        if let UiEvent::Clicked(id) = event {
            on_click(id);
        }
    }
}

/// Draw the widgets onto `canvas`. Draw anything of your own afterwards.
pub fn render<const NODES: usize>(gui: &mut Context<NODES>, canvas: &mut Canvas) {
    if let Err(error) = gui.render(canvas) {
        debug!("GUI render failed: {:?}", error);
    }
}
