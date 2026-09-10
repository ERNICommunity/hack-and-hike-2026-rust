//! Reusable drawing primitives shared by semantic views.
//!
//! KDL owns geometry. These helpers deliberately use native-resolution bitmap
//! fonts instead of scaling tiny glyphs: at 320x240 this gives crisper text with
//! predictable no_std cost while keeping dense telemetry bounded.

use embedded_graphics::{
    mono_font::{ascii::{FONT_6X12, FONT_7X13, FONT_8X13_BOLD}, MonoTextStyle},
    pixelcolor::{raw::RawU16, Rgb565},
    prelude::*,
    primitives::{PrimitiveStyle, Rectangle},
    text::{Baseline, Text},
};
use embedded_gui::Rect;

use crate::theme;

use super::super::gui::GuiFramebuffer;

pub(super) const BODY_LINE_HEIGHT: i32 = 13;
pub(super) const DENSE_LINE_HEIGHT: i32 = 12;

pub(super) fn white() -> Rgb565 {
    raw_color(theme::WHITE_RGB565)
}

pub(super) fn black() -> Rgb565 {
    raw_color(theme::BLACK_RGB565)
}

pub(super) fn dark_blue() -> Rgb565 {
    raw_color(theme::DARK_BLUE_RGB565)
}

pub(super) fn light_blue() -> Rgb565 {
    raw_color(theme::LIGHT_BLUE_RGB565)
}

pub(super) fn dark_gray() -> Rgb565 {
    raw_color(theme::DARK_GRAY_RGB565)
}

pub(super) fn light_gray() -> Rgb565 {
    raw_color(theme::LIGHT_GRAY_RGB565)
}

fn raw_color(raw: u16) -> Rgb565 {
    Rgb565::from(RawU16::new(raw))
}

pub(super) fn fill_rect(frame: &mut GuiFramebuffer, rect: Rect, color: Rgb565) {
    fill_box(frame, rect.x, rect.y, rect.w as u32, rect.h as u32, color);
}

pub(super) fn fill_box(
    frame: &mut GuiFramebuffer,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    color: Rgb565,
) {
    if width == 0 || height == 0 {
        return;
    }
    let _ = Rectangle::new(Point::new(x, y), Size::new(width, height))
        .into_styled(PrimitiveStyle::with_fill(color))
        .draw(frame);
}

pub(super) fn hline(
    frame: &mut GuiFramebuffer,
    x: i32,
    y: i32,
    width: u32,
    color: Rgb565,
) {
    fill_box(frame, x, y, width, 1, color);
}

pub(super) fn vline(
    frame: &mut GuiFramebuffer,
    x: i32,
    y: i32,
    height: u32,
    color: Rgb565,
) {
    fill_box(frame, x, y, 1, height, color);
}

pub(super) fn draw_title(
    frame: &mut GuiFramebuffer,
    text: &str,
    x: i32,
    y: i32,
    color: Rgb565,
) {
    let style = MonoTextStyle::new(&FONT_8X13_BOLD, color);
    let _ = Text::with_baseline(text, Point::new(x, y), style, Baseline::Top).draw(frame);
}

pub(super) fn draw_body(
    frame: &mut GuiFramebuffer,
    text: &str,
    x: i32,
    y: i32,
    color: Rgb565,
) {
    let style = MonoTextStyle::new(&FONT_7X13, color);
    let _ = Text::with_baseline(text, Point::new(x, y), style, Baseline::Top).draw(frame);
}

pub(super) fn draw_dense(
    frame: &mut GuiFramebuffer,
    text: &str,
    x: i32,
    y: i32,
    color: Rgb565,
) {
    let style = MonoTextStyle::new(&FONT_6X12, color);
    let _ = Text::with_baseline(text, Point::new(x, y), style, Baseline::Top).draw(frame);
}

pub(super) fn draw_centered_body(
    frame: &mut GuiFramebuffer,
    rect: Rect,
    text: &str,
    color: Rgb565,
) {
    let width = text.len() as i32 * 7;
    let x = rect.x + ((rect.w as i32 - width) / 2).max(0);
    draw_body(frame, text, x, rect.y, color);
}
