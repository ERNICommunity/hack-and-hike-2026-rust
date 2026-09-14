//! Touch-friendly widgets drawn with `embedded-graphics`.
//!
//! `embedded-gui` provides buttons and labels; its slider cannot be dragged
//! with a finger and is too small for a touch screen, so the slider lives
//! here.

use embedded_graphics::{
    prelude::*,
    primitives::{Circle, PrimitiveStyle, Rectangle},
};

use crate::capabilities::touch::TouchEvent;

use super::{Canvas, theme};

/// Diameter of the round handle, large enough to see under a finger.
const THUMB_DIAMETER: u32 = 22;
/// Diameter of the white dot inside the handle.
const THUMB_HOLE_DIAMETER: u32 = 12;
/// Height of the bar the handle slides along.
const TRACK_HEIGHT: u32 = 8;
/// Touches this far outside the slider still count.
const HIT_MARGIN: u32 = 8;

/// A horizontal slider over an integer range, drawn inside a fixed rectangle.
///
/// The slider does not store its value: the application keeps the value,
/// passes touches to [`Slider::handle_touch`] and draws the current value
/// with [`Slider::draw`].
///
/// ```ignore
/// let mut slider = Slider::new(Rectangle::new(Point::new(20, 100), Size::new(280, 40)), 0, 100);
///
/// // for every touch event:
/// if let Some(new_value) = slider.handle_touch(event) {
///     value = new_value;
/// }
/// // when drawing:
/// slider.draw(&mut canvas, value);
/// ```
#[derive(Clone, Copy)]
pub struct Slider {
    /// Where the slider is drawn, in canvas coordinates.
    area: Rectangle,
    /// The value at the left end.
    min: i32,
    /// The value at the right end.
    max: i32,
    /// Whether the current touch started on the slider.
    dragging: bool,
}

impl Slider {
    /// A slider drawn inside `area`, from `min` on the left to `max` on the
    /// right.
    pub const fn new(area: Rectangle, min: i32, max: i32) -> Self {
        Self {
            area,
            min,
            max,
            dragging: false,
        }
    }

    /// Feed a touch event, in the coordinates of the canvas the slider is
    /// drawn on. Returns the new value while the finger presses, drags or
    /// releases on this slider, and `None` for touches elsewhere.
    ///
    /// A drag that starts on the slider keeps controlling it even when the
    /// finger strays outside.
    pub fn handle_touch(&mut self, event: TouchEvent) -> Option<i32> {
        match event {
            TouchEvent::Pressed(point) if self.hit_area().contains(point) => {
                self.dragging = true;
                Some(self.value_at(point.x))
            }
            TouchEvent::Moved(point) if self.dragging => Some(self.value_at(point.x)),
            TouchEvent::Released(point) if self.dragging => {
                self.dragging = false;
                Some(self.value_at(point.x))
            }
            _ => None,
        }
    }

    /// Draw the slider at `value`, clamped to its range.
    pub fn draw(&self, canvas: &mut Canvas, value: i32) {
        canvas.fill(self.area, theme::WHITE);

        let (left, right) = self.track_bounds();
        let center_y = self.area.center().y;
        let track_y = center_y - TRACK_HEIGHT as i32 / 2;
        let thumb_x = self.thumb_x(value);
        let track = |from: i32, to: i32| {
            Rectangle::new(
                Point::new(from, track_y),
                Size::new((to - from + 1) as u32, TRACK_HEIGHT),
            )
        };

        canvas.fill(track(left, right), theme::LIGHT_GRAY);
        canvas.fill(track(left, thumb_x), theme::DARK_BLUE);
        let thumb = Point::new(thumb_x, center_y);
        let Ok(()) = Circle::with_center(thumb, THUMB_DIAMETER)
            .into_styled(PrimitiveStyle::with_fill(theme::DARK_BLUE))
            .draw(canvas);
        let Ok(()) = Circle::with_center(thumb, THUMB_HOLE_DIAMETER)
            .into_styled(PrimitiveStyle::with_fill(theme::WHITE))
            .draw(canvas);
    }

    /// The area where a press grabs the slider: a little larger than the
    /// slider itself.
    fn hit_area(&self) -> Rectangle {
        self.area.offset(HIT_MARGIN as i32)
    }

    /// Leftmost and rightmost thumb centre positions.
    fn track_bounds(&self) -> (i32, i32) {
        let radius = THUMB_DIAMETER as i32 / 2;
        let left = self.area.top_left.x + radius;
        let right = self.area.top_left.x + self.area.size.width as i32 - radius - 1;
        (left, right.max(left + 1))
    }

    /// The value for a finger at column `x`, rounded to the nearest integer.
    fn value_at(&self, x: i32) -> i32 {
        let (left, right) = self.track_bounds();
        let span = right - left;
        let offset = x.clamp(left, right) - left;
        self.min + (offset * (self.max - self.min) + span / 2) / span
    }

    /// The column of the handle's centre for `value`.
    fn thumb_x(&self, value: i32) -> i32 {
        let (left, right) = self.track_bounds();
        let range = (self.max - self.min).max(1);
        let offset = value.clamp(self.min, self.max) - self.min;
        left + (offset * (right - left) + range / 2) / range
    }
}
