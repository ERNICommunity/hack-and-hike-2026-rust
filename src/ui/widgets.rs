//! Touch-friendly widgets drawn with `embedded-graphics`.
//!
//! `embedded-gui` provides buttons and labels; its slider cannot be dragged
//! with a finger and is too small for a touch screen, so the slider lives here.

use embedded_graphics::{
    prelude::*,
    primitives::{Circle, PrimitiveStyle},
};
use embedded_gui::Rect;

use super::{
    common,
    gui::{GuiFramebuffer, Pointer, PointerPhase},
    theme,
};

const THUMB_DIAMETER: i32 = 22;
const THUMB_RADIUS: i32 = THUMB_DIAMETER / 2;
const THUMB_HOLE_DIAMETER: i32 = 12;
const TRACK_HEIGHT: u32 = 8;
/// Touches this far outside the slider still count.
const HIT_MARGIN: i32 = 8;

/// A horizontal slider over an integer range, drawn inside a fixed rectangle.
#[derive(Clone, Copy)]
pub struct Slider {
    rect: Rect,
    min: i32,
    max: i32,
    dragging: bool,
}

impl Slider {
    pub const fn new(rect: Rect, min: i32, max: i32) -> Self {
        Self {
            rect,
            min,
            max,
            dragging: false,
        }
    }

    /// Feed a touch event. Returns the new value while the finger presses,
    /// drags or releases on this slider.
    pub fn handle_pointer(&mut self, pointer: Pointer) -> Option<i32> {
        match pointer.phase {
            PointerPhase::Pressed if self.contains(pointer) => {
                self.dragging = true;
                Some(self.value_at(pointer.x))
            }
            PointerPhase::Moved if self.dragging => Some(self.value_at(pointer.x)),
            PointerPhase::Released if self.dragging => {
                self.dragging = false;
                Some(self.value_at(pointer.x))
            }
            PointerPhase::Released => {
                self.dragging = false;
                None
            }
            _ => None,
        }
    }

    pub fn draw(&self, frame: &mut GuiFramebuffer, value: i32) {
        common::fill(frame, self.rect, theme::WHITE);

        let (left, right) = self.track_bounds();
        let center_y = self.rect.y + self.rect.h as i32 / 2;
        let track_y = center_y - TRACK_HEIGHT as i32 / 2;
        let thumb_x = self.thumb_x(value);

        common::fill(
            frame,
            Rect::new(left, track_y, (right - left + 1) as u32, TRACK_HEIGHT),
            theme::LIGHT_GRAY,
        );
        common::fill(
            frame,
            Rect::new(left, track_y, (thumb_x - left + 1) as u32, TRACK_HEIGHT),
            theme::DARK_BLUE,
        );
        let _ = Circle::with_center(Point::new(thumb_x, center_y), THUMB_DIAMETER as u32)
            .into_styled(PrimitiveStyle::with_fill(theme::DARK_BLUE))
            .draw(frame);
        let _ = Circle::with_center(Point::new(thumb_x, center_y), THUMB_HOLE_DIAMETER as u32)
            .into_styled(PrimitiveStyle::with_fill(theme::WHITE))
            .draw(frame);
    }

    fn contains(&self, pointer: Pointer) -> bool {
        let rect = self.rect;
        (rect.x - HIT_MARGIN..rect.x + rect.w as i32 + HIT_MARGIN).contains(&pointer.x)
            && (rect.y - HIT_MARGIN..rect.y + rect.h as i32 + HIT_MARGIN).contains(&pointer.y)
    }

    /// Leftmost and rightmost thumb centre positions.
    fn track_bounds(&self) -> (i32, i32) {
        let left = self.rect.x + THUMB_RADIUS;
        let right = self.rect.x + self.rect.w as i32 - THUMB_RADIUS - 1;
        (left, right.max(left + 1))
    }

    fn value_at(&self, x: i32) -> i32 {
        let (left, right) = self.track_bounds();
        let span = right - left;
        let offset = x.clamp(left, right) - left;
        self.min + (offset * (self.max - self.min) + span / 2) / span
    }

    fn thumb_x(&self, value: i32) -> i32 {
        let (left, right) = self.track_bounds();
        let range = (self.max - self.min).max(1);
        let offset = value.clamp(self.min, self.max) - self.min;
        left + (offset * (right - left) + range / 2) / range
    }
}
