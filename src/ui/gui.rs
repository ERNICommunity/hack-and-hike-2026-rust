//! The connection between `embedded-gui` and the rest of the firmware.
//!
//! A screen describes its layout in a KDL file next to its code. KDL is a
//! small document language. The `embedded_gui::include_gui!` macro turns the
//! file into Rust code at compile time. At run time, the screen:
//!
//! 1. creates a [`Context`] once with [`context`], and builds the layout
//!    into it,
//! 2. gets the rectangles of empty slots with [`slot`], to draw its own
//!    content there,
//! 3. passes touch events to the widgets with [`click_buttons`],
//! 4. draws the widgets onto a [`Canvas`] with [`render`], and then draws
//!    its own content on top.
//!
//! See `src/bin/demo/screens/settings/` for a complete example.

use embedded_graphics::{
    prelude::{Point, Size},
    primitives::Rectangle,
};
use embedded_gui::{
    GuiContext, InputEvent, PointerButton, PointerState, PropertyKey, PropertyValue, Rect, Style,
    UiEvent, WidgetId,
};
use log::debug;

use crate::{board::psram, capabilities::touch::TouchEvent};

use super::Canvas;

/// UI events that can wait in the context's queue. [`click_buttons`] reads
/// them after each touch event. One tap creates about nine.
pub const EVENTS: usize = 16;
/// Areas that `embedded-gui` remembers as needing a redraw. The canvas finds
/// its own changes, so this only needs to be more than zero.
pub const DIRTY_RECTS: usize = 8;

/// The `embedded-gui` context of one screen, with room for `NODES` widgets.
pub type Context<const NODES: usize> = GuiContext<'static, NODES, EVENTS, DIRTY_RECTS>;

/// Create the GUI context of a screen of `width` x `height` pixels, in PSRAM.
/// The memory is never freed.
///
/// A context is about 20 KiB. That is too large for a struct on a task's
/// stack.
///
/// # Panics
///
/// When PSRAM does not have enough free memory.
pub fn context<const NODES: usize>(width: u32, height: u32) -> &'static mut Context<NODES> {
    psram::leaked_value(|| Context::new(Rect::new(0, 0, width, height)))
}

/// The rectangle a KDL node occupies, in context coordinates.
///
/// # Panics
///
/// When the id is unknown. The layout is fixed at compile time, so a missing
/// node is a programming error.
pub fn slot<const NODES: usize>(gui: &Context<NODES>, id: WidgetId) -> Rectangle {
    let rect = gui_rect(gui, id);
    Rectangle::new(Point::new(rect.x, rect.y), Size::new(rect.w, rect.h))
}

/// The rectangle of node `id` in `embedded-gui`'s own type.
fn gui_rect<const NODES: usize>(gui: &Context<NODES>, id: WidgetId) -> Rect {
    gui.absolute_rect(id)
        .expect("every KDL node has a rectangle after build()")
}

/// Put a value label (a caption on the left, a number on the right) into the
/// slot of the KDL node `id`. Change the number with [`set_value`].
///
/// KDL files cannot describe value labels, so screens add them in code.
///
/// # Panics
///
/// When the id is unknown, or when the context has no room for another
/// widget. In that case, make its `NODES` larger.
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

/// Change the text of a label or button. The text must be `'static`, which
/// usually means a string literal. When `id` is not a label or button,
/// nothing changes.
pub fn set_text<const NODES: usize>(gui: &mut Context<NODES>, id: WidgetId, text: &'static str) {
    if let Err(error) = gui.set_widget_property(id, PropertyKey::Text, PropertyValue::Str(text)) {
        debug!("GUI text update failed: {:?}", error);
    }
}

/// Change the number of a value label from [`add_value_label`]. When `id` is
/// not a value label, nothing changes.
pub fn set_value<const NODES: usize>(gui: &mut Context<NODES>, id: WidgetId, value: i32) {
    if let Err(error) = gui.set_value_label(id, value) {
        debug!("GUI value update failed: {:?}", error);
    }
}

/// Pass a touch event to the widgets. Call `on_click` with the id of every
/// button that the touch clicked. The position of `event` must be in the
/// coordinates of the context.
///
/// ```ignore
/// let mut clicked = None;
/// gui::click_buttons(self.gui, event, |id| clicked = Some(id));
/// if clicked == Some(self.play_button) { /* ... */ }
/// ```
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

/// Draw the widgets onto `canvas`. Draw your own content after this call.
pub fn render<const NODES: usize>(gui: &mut Context<NODES>, canvas: &mut Canvas) {
    if let Err(error) = gui.render(canvas) {
        debug!("GUI render failed: {:?}", error);
    }
}
