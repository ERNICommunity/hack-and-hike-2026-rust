//! The project colour palette.
//!
//! These are ordinary `embedded-graphics` colours; `Rgb565::RED`,
//! `Rgb565::new(31, 63, 31)` and friends work everywhere too.

use embedded_graphics::pixelcolor::Rgb565;

/// An `0xRRGGBB` colour as RGB565.
pub const fn rgb(rgb: u32) -> Rgb565 {
    let red = ((rgb >> 16) & 0xFF) as u8;
    let green = ((rgb >> 8) & 0xFF) as u8;
    let blue = (rgb & 0xFF) as u8;
    Rgb565::new(red >> 3, green >> 2, blue >> 3)
}

pub const DARK_BLUE: Rgb565 = rgb(0x033778);
pub const LIGHT_BLUE: Rgb565 = rgb(0x00AADB);
pub const DARK_GRAY: Rgb565 = rgb(0x515455);
pub const LIGHT_GRAY: Rgb565 = rgb(0xB1B0B1);
pub const WHITE: Rgb565 = rgb(0xFFFFFF);
/// Text colour: a soft black.
pub const CHARCOAL: Rgb565 = rgb(0x3C3C3B);
