//! The colour palette, as `embedded-graphics` colours and as raw RGB565 pixels.

use embedded_graphics::pixelcolor::Rgb565;

use crate::capabilities::display::Pixel;

/// Convert a 24-bit `0xRRGGBB` colour to RGB565.
const fn rgb565(rgb: u32) -> Pixel {
    let red = ((rgb >> 16) & 0xff) as u16;
    let green = ((rgb >> 8) & 0xff) as u16;
    let blue = (rgb & 0xff) as u16;
    ((red >> 3) << 11) | ((green >> 2) << 5) | (blue >> 3)
}

const fn color(rgb: u32) -> Rgb565 {
    let red = ((rgb >> 16) & 0xff) as u8;
    let green = ((rgb >> 8) & 0xff) as u8;
    let blue = (rgb & 0xff) as u8;
    Rgb565::new(red >> 3, green >> 2, blue >> 3)
}

const DARK_BLUE_RGB: u32 = 0x033778;
const LIGHT_BLUE_RGB: u32 = 0x00AADB;
const DARK_GRAY_RGB: u32 = 0x515455;
const LIGHT_GRAY_RGB: u32 = 0xB1B0B1;
const WHITE_RGB: u32 = 0xFFFFFF;
const CHARCOAL_RGB: u32 = 0x3C3C3B;

pub const DARK_BLUE: Rgb565 = color(DARK_BLUE_RGB);
pub const LIGHT_BLUE: Rgb565 = color(LIGHT_BLUE_RGB);
pub const DARK_GRAY: Rgb565 = color(DARK_GRAY_RGB);
pub const LIGHT_GRAY: Rgb565 = color(LIGHT_GRAY_RGB);
pub const WHITE: Rgb565 = color(WHITE_RGB);
/// Text colour: a soft black.
pub const CHARCOAL: Rgb565 = color(CHARCOAL_RGB);

/// The same palette as raw pixels for `Surface::render_scanlines`.
pub mod pixel {
    use super::{Pixel, rgb565};

    pub const DARK_BLUE: Pixel = rgb565(super::DARK_BLUE_RGB);
    pub const LIGHT_BLUE: Pixel = rgb565(super::LIGHT_BLUE_RGB);
    pub const DARK_GRAY: Pixel = rgb565(super::DARK_GRAY_RGB);
    pub const LIGHT_GRAY: Pixel = rgb565(super::LIGHT_GRAY_RGB);
    pub const WHITE: Pixel = rgb565(super::WHITE_RGB);
    pub const CHARCOAL: Pixel = rgb565(super::CHARCOAL_RGB);
}
