//! Text helpers on top of `embedded-graphics`.
//!
//! Shapes need no helpers: `Rectangle`, `Line`, `Circle` and the rest of
//! `embedded_graphics::primitives` draw straight onto a [`Canvas`]. Text
//! uses the native-resolution bitmap fonts: at 320x240 they are crisper than
//! scaled glyphs and their cost is predictable.

use embedded_graphics::{
    mono_font::{
        MonoFont, MonoTextStyle,
        ascii::{FONT_6X12, FONT_7X13, FONT_8X13_BOLD},
    },
    pixelcolor::Rgb565,
    prelude::*,
    primitives::Rectangle,
    text::{Alignment, Baseline, Text, TextStyleBuilder},
};

use super::Canvas;

pub const TITLE_FONT: &MonoFont<'static> = &FONT_8X13_BOLD;
pub const BODY_FONT: &MonoFont<'static> = &FONT_7X13;
pub const DENSE_FONT: &MonoFont<'static> = &FONT_6X12;

/// Height of one line of `BODY_FONT` text.
pub const BODY_LINE_HEIGHT: i32 = 13;
/// Height of one line of `DENSE_FONT` text.
pub const DENSE_LINE_HEIGHT: i32 = 12;

/// Draw `text` with its top-left corner at `origin`.
pub fn text(canvas: &mut Canvas, text: &str, origin: Point, font: &MonoFont<'_>, color: Rgb565) {
    let style = MonoTextStyle::new(font, color);
    let Ok(_) = Text::with_baseline(text, origin, style, Baseline::Top).draw(canvas);
}

/// Draw `text` centered inside `area`.
pub fn centered_text(
    canvas: &mut Canvas,
    area: Rectangle,
    text: &str,
    font: &MonoFont<'_>,
    color: Rgb565,
) {
    let style = MonoTextStyle::new(font, color);
    let layout = TextStyleBuilder::new()
        .alignment(Alignment::Center)
        .baseline(Baseline::Middle)
        .build();
    let Ok(_) = Text::with_text_style(text, area.center(), style, layout).draw(canvas);
}

/// Writes consecutive lines of `BODY_FONT` text downwards from a start point.
pub struct Lines<'a> {
    canvas: &'a mut Canvas,
    next: Point,
}

impl<'a> Lines<'a> {
    pub fn new(canvas: &'a mut Canvas, origin: Point) -> Self {
        Self {
            canvas,
            next: origin,
        }
    }

    pub fn line(&mut self, text: &str, color: Rgb565) {
        self::text(self.canvas, text, self.next, BODY_FONT, color);
        self.next.y += BODY_LINE_HEIGHT;
    }

    /// Leave one line empty.
    pub fn skip(&mut self) {
        self.next.y += BODY_LINE_HEIGHT;
    }
}
