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
///
/// The canvas keeps track of what changed, so `show` only sends the changed
/// rectangle and `clear` with the same colour as last time only repaints
/// what was drawn since. A full-screen transfer takes about 30 ms; redrawing
/// a number in a corner takes well under one.
pub struct Canvas {
    pixels: &'static mut [Rgb565],
    size: Size,
    /// The colour of every pixel outside `drawn`, if the canvas was cleared.
    background: Option<Rgb565>,
    /// Pixels drawn since the last `clear`.
    drawn: Option<Rectangle>,
    /// Pixels changed since the last `show`.
    changed: Option<Rectangle>,
}

impl Canvas {
    /// A white canvas of `size` pixels.
    pub fn new(size: Size) -> Self {
        let count = size.width as usize * size.height as usize;
        assert!(count != 0, "a canvas needs at least one pixel");
        Self {
            pixels: storage::leaked_slice(count, Rgb565::WHITE),
            size,
            background: Some(Rgb565::WHITE),
            drawn: None,
            changed: Some(Rectangle::new(Point::zero(), size)),
        }
    }

    pub const fn size(&self) -> Size {
        self.size
    }

    /// Fill the whole canvas with one colour.
    pub fn clear(&mut self, color: Rgb565) {
        if self.background == Some(color) {
            // Only what was drawn since the last clear differs from `color`.
            if let Some(drawn) = self.drawn {
                self.paint(drawn, color);
                self.changed = union(self.changed, drawn);
            }
        } else {
            self.pixels.fill(color);
            self.changed = Some(self.bounding_box());
        }
        self.background = Some(color);
        self.drawn = None;
    }

    /// Fill a rectangle; the part outside the canvas is ignored.
    pub fn fill(&mut self, area: Rectangle, color: Rgb565) {
        let area = area.intersection(&self.bounding_box());
        self.paint(area, color);
        self.touch(area);
    }

    /// Set one pixel; a point outside the canvas is ignored.
    pub fn set(&mut self, point: Point, color: Rgb565) {
        if self.bounding_box().contains(point) {
            self.pixels[self.index(point)] = color;
            self.touch(Rectangle::new(point, Size::new(1, 1)));
        }
    }

    /// Copy what changed since the last call to `surface`, which must have
    /// the same size as the canvas.
    pub fn show(&mut self, surface: &mut Surface<'_>) {
        assert_eq!(
            surface.size(),
            self.size,
            "the surface must be the size of the canvas"
        );
        if let Some(changed) = self.changed.take() {
            let changed = even_columns(changed, self.size);
            surface.subsurface(changed).render_from(&mut Window {
                canvas: self,
                area: changed,
            });
        }
    }

    /// Forget what the panel shows, so the next `show` sends the whole
    /// canvas. Call this after drawing on the surface without the canvas.
    pub fn invalidate(&mut self) {
        self.changed = Some(self.bounding_box());
    }

    fn index(&self, point: Point) -> usize {
        point.y as usize * self.size.width as usize + point.x as usize
    }

    /// Fill `area`, which must lie inside the canvas, without bookkeeping.
    fn paint(&mut self, area: Rectangle, color: Rgb565) {
        let width = area.size.width as usize;
        for y in area.rows() {
            let start = self.index(Point::new(area.top_left.x, y));
            self.pixels[start..start + width].fill(color);
        }
    }

    /// Record that `area` (inside the canvas) was drawn on.
    fn touch(&mut self, area: Rectangle) {
        if area.is_zero_sized() {
            return;
        }
        self.drawn = union(self.drawn, area);
        self.changed = union(self.changed, area);
    }
}

/// Widen `area` so that it starts on an even column and spans an even number
/// of columns, staying inside a canvas of `size`. Every row then transfers
/// as whole 32-bit words, which the SPI DMA handles exactly.
fn even_columns(area: Rectangle, size: Size) -> Rectangle {
    let left = area.top_left.x & !1;
    let right = (area.top_left.x + area.size.width as i32 + 1) & !1;
    let right = right.min(size.width as i32);
    Rectangle::new(
        Point::new(left, area.top_left.y),
        Size::new((right - left) as u32, area.size.height),
    )
}

/// The smallest rectangle containing both.
fn union(a: Option<Rectangle>, b: Rectangle) -> Option<Rectangle> {
    let Some(a) = a else {
        return Some(b);
    };
    let (Some(a_end), Some(b_end)) = (a.bottom_right(), b.bottom_right()) else {
        return Some(if a.is_zero_sized() { b } else { a });
    };
    let top_left = a.top_left.component_min(b.top_left);
    let bottom_right = a_end.component_max(b_end);
    Some(Rectangle::with_corners(top_left, bottom_right))
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
        let mut touched: Option<Rectangle> = None;
        for Pixel(point, color) in pixels {
            if bounds.contains(point) {
                self.pixels[self.index(point)] = color;
                touched = union(touched, Rectangle::new(point, Size::new(1, 1)));
            }
        }
        if let Some(touched) = touched {
            self.touch(touched);
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

/// The rows of one rectangle of a canvas, as the display wants them.
struct Window<'a> {
    canvas: &'a Canvas,
    area: Rectangle,
}

impl ScanlineSource for Window<'_> {
    fn fill_row(&mut self, y: usize, row: &mut [u8]) {
        let start = self
            .canvas
            .index(self.area.top_left + Point::new(0, y as i32));
        let pixels = &self.canvas.pixels[start..start + row.len() / BYTES_PER_PIXEL];
        for (bytes, pixel) in row.chunks_exact_mut(BYTES_PER_PIXEL).zip(pixels) {
            bytes.copy_from_slice(&RawU16::from(*pixel).into_inner().to_be_bytes());
        }
    }
}
