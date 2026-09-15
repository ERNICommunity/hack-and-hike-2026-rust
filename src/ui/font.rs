//! Top-anchored fonts for `embedded-gui`.
//!
//! `embedded-gui` places `embedded-graphics` fonts on their baseline (the
//! line that letters sit on). That puts every character too high, above the
//! rectangle it belongs to. These adapters place characters by their top-left
//! corner instead. So the text of widgets lines up with the layout, in the
//! same way as with [`super::common::text`].

use embedded_graphics::{
    Drawable as _, Pixel,
    draw_target::DrawTarget,
    geometry::{OriginDimensions, Point, Size},
    mono_font::{MonoFont, MonoTextStyle},
    pixelcolor::BinaryColor,
    text::{Baseline, Text},
};
use embedded_gui::{Font, FontId};

use super::common;

/// A monospace font that places each character by its top-left corner.
pub struct TopAnchored(&'static MonoFont<'static>);

impl TopAnchored {
    /// Reference this font from a widget [`embedded_gui::Style`].
    pub fn id(&'static self) -> FontId {
        FontId::Dynamic(self)
    }
}

/// [`common::TITLE_FONT`] for `embedded-gui` widgets.
pub static TITLE: TopAnchored = TopAnchored(common::TITLE_FONT);
/// [`common::BODY_FONT`] for `embedded-gui` widgets.
pub static BODY: TopAnchored = TopAnchored(common::BODY_FONT);

impl Font for TopAnchored {
    fn advance(&self) -> u32 {
        self.0.character_size.width + self.0.character_spacing
    }

    fn line_height(&self) -> u32 {
        self.0.character_size.height
    }

    fn draw_glyph(&self, ch: char, draw_pixel: &mut dyn FnMut(i32, i32)) {
        let style = MonoTextStyle::new(self.0, BinaryColor::On);
        let mut buffer = [0u8; 4];
        let text = ch.encode_utf8(&mut buffer);
        let mut glyph = Glyph { draw_pixel };
        let Ok(_) = Text::with_baseline(text, Point::zero(), style, Baseline::Top).draw(&mut glyph);
    }
}

/// A draw target that forwards the lit pixels of one glyph to
/// `embedded-gui`'s pixel callback. Its size is unlimited, so nothing is
/// clipped here; `embedded-gui` clips afterwards.
struct Glyph<'a> {
    /// Called with the `(x, y)` of each lit pixel, relative to the glyph's
    /// top-left corner.
    draw_pixel: &'a mut dyn FnMut(i32, i32),
}

impl OriginDimensions for Glyph<'_> {
    fn size(&self) -> Size {
        Size::new(u32::MAX, u32::MAX)
    }
}

impl DrawTarget for Glyph<'_> {
    type Color = BinaryColor;
    type Error = core::convert::Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        for Pixel(position, color) in pixels {
            if color.is_on() {
                (self.draw_pixel)(position.x, position.y);
            }
        }
        Ok(())
    }
}
