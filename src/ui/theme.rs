//! The project colour palette.
//!
//! These are normal `embedded-graphics` colours. Other colours, such as
//! `Rgb565::RED` or `Rgb565::new(31, 63, 31)`, work everywhere too.
//!
//! RGB565 has 32 levels of red and blue, and 64 levels of green. So a colour
//! that you choose on a computer screen looks similar on the panel, but not
//! the same.

use embedded_graphics::pixelcolor::Rgb565;

/// Convert an `0xRRGGBB` colour, as written on the web, to RGB565. The low
/// bits of each channel are dropped.
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
