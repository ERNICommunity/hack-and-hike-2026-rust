//! Glue between `embedded-gui` and the display.
//!
//! A graphical screen describes its layout in a KDL file, builds it into a
//! [`Context`] once, and renders into a [`GuiSurface`] framebuffer that is
//! then copied to the LCD. Touches reach the widgets through [`Pointer`].

use embedded_graphics::{
    pixelcolor::{Rgb565, raw::RawU16},
    prelude::{DrawTarget as _, Point, RawData as _},
};
use embedded_gui::{
    EndianCorrectedBuffer, EndianCorrection, FrameBuf, GuiContext, InputEvent, PointerButton,
    PointerState, PropertyKey, PropertyValue, Rect, UiEvent, WidgetId,
};
use log::debug;

use crate::{
    capabilities::display::{ScanlineSource, Surface},
    support::memory::storage,
};

use super::theme;

pub type GuiFramebufferBackend = EndianCorrectedBuffer<'static, Rgb565>;
pub type GuiFramebuffer = FrameBuf<Rgb565, GuiFramebufferBackend>;

/// UI events one touch may queue before the screen drains them; a single tap
/// produces about nine.
pub const EVENTS: usize = 16;
pub const DIRTY_RECTS: usize = 8;

/// The `embedded-gui` context of one screen with room for `NODES` widgets.
pub type Context<const NODES: usize> = GuiContext<'static, NODES, EVENTS, DIRTY_RECTS>;

/// Allocate a screen's GUI context in PSRAM; it lives for the rest of the run.
pub fn context<const NODES: usize>(width: usize, height: usize) -> &'static mut Context<NODES> {
    let width = u32::try_from(width).expect("screen width fits u32");
    let height = u32::try_from(height).expect("screen height fits u32");
    storage::leaked_value_with(|| Context::new(Rect::new(0, 0, width, height)))
}

/// The rectangle a KDL node occupies.
///
/// Panics when the id is unknown: the layout is fixed at compile time, so a
/// missing node is a programming error, not a runtime condition.
pub fn slot<const NODES: usize>(gui: &Context<NODES>, id: WidgetId) -> Rect {
    gui.absolute_rect(id)
        .expect("every KDL node has a rectangle after build()")
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerPhase {
    Pressed,
    Moved,
    Released,
}

/// A touch in the coordinates of the screen it lands on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pointer {
    pub x: i32,
    pub y: i32,
    pub phase: PointerPhase,
}

impl Pointer {
    fn input_event(self) -> InputEvent {
        let state = match self.phase {
            PointerPhase::Pressed => PointerState::Pressed,
            PointerPhase::Moved => PointerState::Moved,
            PointerPhase::Released => PointerState::Released,
        };
        InputEvent::Pointer {
            x: self.x,
            y: self.y,
            state,
            button: PointerButton::Primary,
        }
    }
}

/// Deliver a touch to the widgets and call `on_click` for every button it
/// clicked.
pub fn click_buttons<const NODES: usize>(
    gui: &mut Context<NODES>,
    pointer: Pointer,
    mut on_click: impl FnMut(WidgetId),
) {
    if let Err(error) = gui.handle_input(pointer.input_event()) {
        debug!("GUI input failed: {:?}", error);
    }
    while let Some(event) = gui.pop_event() {
        if let UiEvent::Clicked(id) = event {
            on_click(id);
        }
    }
}

/// One framebuffer, the size of the content area, shared by all screens.
pub struct GuiSurface {
    framebuffer: GuiFramebuffer,
    width: usize,
    height: usize,
}

impl GuiSurface {
    pub fn new(width: usize, height: usize) -> Self {
        assert!(width != 0 && height != 0);
        let pixels = storage::leaked_filled_slice(width * height, theme::WHITE);
        let backend = EndianCorrectedBuffer::new(pixels, EndianCorrection::ToBigEndian);
        Self {
            framebuffer: FrameBuf::new(backend, width, height),
            width,
            height,
        }
    }

    /// Render the widgets, let `overlay` draw on top, and show the result.
    pub fn present<const NODES: usize>(
        &mut self,
        surface: &mut Surface<'_>,
        gui: &mut Context<NODES>,
        overlay: impl FnOnce(&mut GuiFramebuffer),
    ) {
        self.present_custom(surface, |frame| {
            if let Err(error) = gui.render(frame) {
                debug!("GUI render failed: {:?}", error);
            }
            overlay(frame);
        });
    }

    /// Show a frame drawn entirely by `draw` on a white background.
    pub fn present_custom(
        &mut self,
        surface: &mut Surface<'_>,
        draw: impl FnOnce(&mut GuiFramebuffer),
    ) {
        debug_assert_eq!(surface.width(), self.width);
        debug_assert_eq!(surface.height(), self.height);

        let _ = self.framebuffer.clear(theme::WHITE);
        draw(&mut self.framebuffer);
        surface.render_from(&mut FramebufferRows {
            framebuffer: &self.framebuffer,
        });
    }
}

/// Streams a rendered framebuffer to the display row by row.
struct FramebufferRows<'a> {
    framebuffer: &'a GuiFramebuffer,
}

impl ScanlineSource for FramebufferRows<'_> {
    fn fill_row(&mut self, y: usize, row: &mut [u8]) {
        let y = i32::try_from(y).expect("framebuffer rows fit in i32");
        for (x, bytes) in (0..).zip(row.chunks_exact_mut(2)) {
            let color = self.framebuffer.get_color_at(Point::new(x, y));
            bytes.copy_from_slice(&RawU16::from(color).into_inner().to_be_bytes());
        }
    }
}
