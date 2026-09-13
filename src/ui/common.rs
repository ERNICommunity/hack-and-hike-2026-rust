//! Small drawing helpers on top of `embedded-graphics`.
//!
//! Text uses the native-resolution bitmap fonts: at 320x240 they are crisper
//! than scaled glyphs and their cost is predictable.

use embedded_graphics::{
    mono_font::{
        MonoFont, MonoTextStyle,
        ascii::{FONT_6X12, FONT_7X13, FONT_8X13_BOLD},
    },
    pixelcolor::Rgb565,
    prelude::*,
    primitives::{PrimitiveStyle, Rectangle},
    text::{Baseline, Text},
};
use embedded_gui::Rect;

use super::gui::GuiFramebuffer;

pub const TITLE_FONT: &MonoFont<'static> = &FONT_8X13_BOLD;
pub const BODY_FONT: &MonoFont<'static> = &FONT_7X13;
pub const DENSE_FONT: &MonoFont<'static> = &FONT_6X12;

/// Height of one line of `BODY_FONT` text.
pub const BODY_LINE_HEIGHT: i32 = 13;
/// Height of one line of `DENSE_FONT` text.
pub const DENSE_LINE_HEIGHT: i32 = 12;

pub fn fill(frame: &mut GuiFramebuffer, rect: Rect, color: Rgb565) {
    if rect.w == 0 || rect.h == 0 {
        return;
    }
    let _ = Rectangle::new(Point::new(rect.x, rect.y), Size::new(rect.w, rect.h))
        .into_styled(PrimitiveStyle::with_fill(color))
        .draw(frame);
}

pub fn outline(frame: &mut GuiFramebuffer, rect: Rect, color: Rgb565) {
    if rect.w == 0 || rect.h == 0 {
        return;
    }
    let _ = Rectangle::new(Point::new(rect.x, rect.y), Size::new(rect.w, rect.h))
        .into_styled(PrimitiveStyle::with_stroke(color, 1))
        .draw(frame);
}

pub fn hline(frame: &mut GuiFramebuffer, x: i32, y: i32, width: u32, color: Rgb565) {
    fill(frame, Rect::new(x, y, width, 1), color);
}

pub fn vline(frame: &mut GuiFramebuffer, x: i32, y: i32, height: u32, color: Rgb565) {
    fill(frame, Rect::new(x, y, 1, height), color);
}

/// Draw `text` with its top-left corner at `origin`.
pub fn text(
    frame: &mut GuiFramebuffer,
    text: &str,
    origin: Point,
    font: &'static MonoFont<'static>,
    color: Rgb565,
) {
    let style = MonoTextStyle::new(font, color);
    let _ = Text::with_baseline(text, origin, style, Baseline::Top).draw(frame);
}

/// Draw `text` centered inside `rect`.
pub fn centered_text(
    frame: &mut GuiFramebuffer,
    rect: Rect,
    text: &str,
    font: &'static MonoFont<'static>,
    color: Rgb565,
) {
    let glyph = font.character_size;
    let width = i32::try_from(text.chars().count()).unwrap_or(0) * glyph.width as i32;
    let x = rect.x + ((rect.w as i32 - width) / 2).max(0);
    let y = rect.y + ((rect.h as i32 - glyph.height as i32) / 2).max(0);
    self::text(frame, text, Point::new(x, y), font, color);
}

/// Writes consecutive lines of `BODY_FONT` text downwards from a start point.
pub struct Lines<'a> {
    frame: &'a mut GuiFramebuffer,
    x: i32,
    y: i32,
}

impl<'a> Lines<'a> {
    pub fn new(frame: &'a mut GuiFramebuffer, origin: Point) -> Self {
        Self {
            frame,
            x: origin.x,
            y: origin.y,
        }
    }

    pub fn line(&mut self, text: &str, color: Rgb565) {
        self::text(
            self.frame,
            text,
            Point::new(self.x, self.y),
            BODY_FONT,
            color,
        );
        self.y += BODY_LINE_HEIGHT;
    }
}
