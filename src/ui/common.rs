//! Fonts and text helpers for `embedded-graphics`.
//!
//! Shapes need no helpers: `Rectangle`, `Line`, `Circle` and the other types
//! in `embedded_graphics::primitives` draw directly onto a [`Canvas`]. Text
//! uses bitmap fonts at their original size. At 320x240, they look sharper
//! than scaled letters, and every character has the same width in pixels.
//!
//! ```ignore
//! common::text(&mut canvas, "SCORE", Point::new(10, 10), common::TITLE_FONT, theme::DARK_BLUE);
//!
//! let mut lines = Lines::new(&mut canvas, Point::new(10, 40));
//! lines.line("First line", theme::CHARCOAL);
//! lines.line("Second line", theme::DARK_GRAY);
//! ```
//!
//! To show numbers, first format them into a buffer of fixed size:
//! `let mut text = ArrayString::<16>::new(); write!(text, "{score}")`.

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

/// Bold, 8x13 pixels per character: titles and emphasis.
pub const TITLE_FONT: &MonoFont<'static> = &FONT_8X13_BOLD;
/// Regular, 7x13 pixels per character: most text. 45 characters fit the
/// width of the screen.
pub const BODY_FONT: &MonoFont<'static> = &FONT_7X13;
/// Small, 6x12 pixels per character: dense lists such as the log.
pub const DENSE_FONT: &MonoFont<'static> = &FONT_6X12;

/// Height of one line of `BODY_FONT` text, in pixels.
pub const BODY_LINE_HEIGHT: i32 = 13;
/// Height of one line of `DENSE_FONT` text, in pixels.
pub const DENSE_LINE_HEIGHT: i32 = 12;

/// Draw `text` with its top-left corner at `origin`.
///
/// Characters that the font does not have (everything that is not ASCII) are
/// drawn as `?`.
pub fn text(canvas: &mut Canvas, text: &str, origin: Point, font: &MonoFont<'_>, color: Rgb565) {
    let style = MonoTextStyle::new(font, color);
    let Ok(_) = Text::with_baseline(text, origin, style, Baseline::Top).draw(canvas);
}

/// Draw `text` in the centre of `area`, both horizontally and vertically.
/// Text that is wider than `area` goes beyond it on both sides.
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

/// Writes lines of [`BODY_FONT`] text, one below the other, from a start
/// point.
pub struct Lines<'a> {
    /// The canvas the lines are drawn on, borrowed until the `Lines` is dropped.
    canvas: &'a mut Canvas,
    /// Where the next line's top-left corner goes.
    next: Point,
}

impl<'a> Lines<'a> {
    /// Start writing at `origin`, the top-left corner of the first line.
    pub fn new(canvas: &'a mut Canvas, origin: Point) -> Self {
        Self {
            canvas,
            next: origin,
        }
    }

    /// Write one line and move down by [`BODY_LINE_HEIGHT`].
    pub fn line(&mut self, text: &str, color: Rgb565) {
        self::text(self.canvas, text, self.next, BODY_FONT, color);
        self.next.y += BODY_LINE_HEIGHT;
    }

    /// Leave one line empty.
    pub fn skip(&mut self) {
        self.next.y += BODY_LINE_HEIGHT;
    }
}
