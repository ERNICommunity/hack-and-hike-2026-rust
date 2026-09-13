//! An off-screen image to draw into, then show on the display in one go.

use core::convert::Infallible;

use embedded_graphics::{
    Pixel,
    pixelcolor::{Rgb565, raw::RawU16},
    prelude::{Dimensions, DrawTarget, Point, RawData as _, RgbColor as _, Size},
    primitives::Rectangle,
};

use crate::{
    capabilities::display::{BYTES_PER_PIXEL, ScanlineSource, Surface},
    support::memory::storage,
};

/// A rectangle of pixels in PSRAM.
///
/// Draw into it with anything from `embedded-graphics` (it is a
/// [`DrawTarget`]), then copy it to the panel with [`Canvas::show`]. Drawing
/// outside the canvas is silently clipped. Allocate one per application, not
/// one per frame: the memory is never freed.
pub struct Canvas {
    pixels: &'static mut [Rgb565],
    size: Size,
}

impl Canvas {
    /// A white canvas of `size` pixels.
    pub fn new(size: Size) -> Self {
        let count = size.width as usize * size.height as usize;
        assert!(count != 0, "a canvas needs at least one pixel");
        Self {
            pixels: storage::leaked_slice(count, Rgb565::WHITE),
            size,
        }
    }

    pub const fn size(&self) -> Size {
        self.size
    }

    /// Fill the whole canvas with one colour.
    pub fn clear(&mut self, color: Rgb565) {
        self.pixels.fill(color);
    }

    /// Fill a rectangle; the part outside the canvas is ignored.
    pub fn fill(&mut self, area: Rectangle, color: Rgb565) {
        let area = area.intersection(&self.bounding_box());
        let width = self.size.width as usize;
        for y in area.rows() {
            let start = y as usize * width + area.top_left.x as usize;
            self.pixels[start..start + area.size.width as usize].fill(color);
        }
    }

    /// Copy the canvas to `surface`, which must have the same size.
    pub fn show(&mut self, surface: &mut Surface<'_>) {
        assert_eq!(
            surface.size(),
            self.size,
            "the surface must be the size of the canvas"
        );
        surface.render_from(self);
    }
}

impl Dimensions for Canvas {
    fn bounding_box(&self) -> Rectangle {
        Rectangle::new(Point::zero(), self.size)
    }
}

impl DrawTarget for Canvas {
    type Color = Rgb565;
    type Error = Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        let bounds = self.bounding_box();
        let width = self.size.width as usize;
        for Pixel(point, color) in pixels {
            if bounds.contains(point) {
                self.pixels[point.y as usize * width + point.x as usize] = color;
            }
        }
        Ok(())
    }

    fn fill_solid(&mut self, area: &Rectangle, color: Self::Color) -> Result<(), Self::Error> {
        self.fill(*area, color);
        Ok(())
    }

    fn clear(&mut self, color: Self::Color) -> Result<(), Self::Error> {
        Canvas::clear(self, color);
        Ok(())
    }
}

impl ScanlineSource for Canvas {
    fn fill_row(&mut self, y: usize, row: &mut [u8]) {
        let start = y * self.size.width as usize;
        let pixels = &self.pixels[start..start + row.len() / BYTES_PER_PIXEL];
        for (bytes, pixel) in row.chunks_exact_mut(BYTES_PER_PIXEL).zip(pixels) {
            bytes.copy_from_slice(&RawU16::from(*pixel).into_inner().to_be_bytes());
        }
    }
}
