//! The project colour palette.
//!
//! These are ordinary `embedded-graphics` colours; `Rgb565::RED`,
//! `Rgb565::new(31, 63, 31)` and friends work everywhere too.
//!
//! RGB565 has 32 levels of red and blue and 64 of green, so a colour picked
//! on a computer screen looks close but not identical on the panel.

use embedded_graphics::pixelcolor::Rgb565;

/// An `0xRRGGBB` colour, as written on the web, converted to RGB565 by
/// dropping the low bits of each channel.
///
/// ```ignore
/// const ORANGE: Rgb565 = theme::rgb(0xFF8000);
/// ```
pub const fn rgb(rgb: u32) -> Rgb565 {
    let red = ((rgb >> 16) & 0xFF) as u8;
    let green = ((rgb >> 8) & 0xFF) as u8;
    let blue = (rgb & 0xFF) as u8;
    Rgb565::new(red >> 3, green >> 2, blue >> 3)
}

/// Primary colour: titles, buttons, the navigation rail.
pub const DARK_BLUE: Rgb565 = rgb(0x033778);
/// Accent colour: highlights and pressed states.
pub const LIGHT_BLUE: Rgb565 = rgb(0x00AADB);
/// Secondary text and hints.
pub const DARK_GRAY: Rgb565 = rgb(0x515455);
/// Inactive elements, such as a slider's empty track.
pub const LIGHT_GRAY: Rgb565 = rgb(0xB1B0B1);
/// Backgrounds.
pub const WHITE: Rgb565 = rgb(0xFFFFFF);
/// Body text: a soft black.
pub const CHARCOAL: Rgb565 = rgb(0x3C3C3B);
