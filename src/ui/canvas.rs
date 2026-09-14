//! An off-screen image to draw into, then show on the display.
//!
//! Sending pixels to the panel is the slow part of drawing: a full frame takes
//! about 31 ms on the SPI bus, whatever the CPU does. [`Canvas`] therefore
//! remembers what the panel already shows and sends only the pixels that
//! differ. An application can redraw its whole picture every time something
//! changes and still update the panel in a millisecond or two.

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

/// Unchanged rows between two changed ones that are still sent in the same
/// window. Programming a new window costs three commands and a fresh DMA
/// transfer; a few rows of unchanged pixels cost less than that.
const MAX_GAP_ROWS: i32 = 4;

/// A rectangle of pixels in PSRAM to draw on with `embedded-graphics`.
///
/// # Drawing
///
/// `Canvas` is a [`DrawTarget`], so every primitive, text style and image of
/// `embedded-graphics` draws onto it. Drawing outside the canvas is clipped.
///
/// ```ignore
/// let mut canvas = Canvas::new(display::SIZE);
///
/// canvas.clear(theme::WHITE);
/// let Ok(()) = Circle::with_center(Point::new(160, 120), 40)
///     .into_styled(PrimitiveStyle::with_fill(theme::LIGHT_BLUE))
///     .draw(&mut canvas);
/// canvas.show(&mut display.surface(display::SCREEN));
/// ```
///
/// Drawing cannot fail, which is why `let Ok(()) = ...` compiles: the error
/// type is [`Infallible`].
///
/// # Showing
///
/// [`Canvas::show`] copies the canvas to the panel, but only the pixels that
/// changed since the last `show`:
///
/// - The canvas keeps a second copy of what the panel shows (the *shadow*)
///   and compares against it, so redrawing the same picture sends nothing.
/// - Changed rows are grouped into a few rectangles, so a number in one
///   corner and a highlight in another do not send the whole screen between
///   them.
/// - [`Canvas::clear`] with the same colour as last time only repaints what
///   was drawn since, instead of every pixel.
///
/// Create a canvas once, before your loop: its memory is never freed.
pub struct Canvas {
    /// What the application drew, row by row.
    pixels: &'static mut [Rgb565],
    /// What the panel shows, valid only when `panel_known` is true.
    shown: &'static mut [Rgb565],
    /// Width and height in pixels; fixed when the canvas is created.
    size: Size,
    /// Whether `shown` matches the panel. False before the first `show` and
    /// after `invalidate`, when something else may have drawn on the panel.
    panel_known: bool,
    /// The colour of every pixel outside `drawn`, if the canvas was cleared.
    background: Option<Rgb565>,
    /// Pixels drawn since the last `clear`.
    drawn: Bounds,
    /// Pixels drawn since the last `show`: the only ones `show` compares.
    changed: Bounds,
}

impl Canvas {
    /// A white canvas of `size` pixels, in PSRAM.
    ///
    /// # Panics
    ///
    /// When `size` has no pixels.
    pub fn new(size: Size) -> Self {
        let count = size.width as usize * size.height as usize;
        assert!(count != 0, "a canvas needs at least one pixel");
        let full = Bounds::of(Rectangle::new(Point::zero(), size));
        Self {
            pixels: storage::leaked_slice(count, Rgb565::WHITE),
            shown: storage::leaked_slice(count, Rgb565::WHITE),
            size,
            panel_known: false,
            background: Some(Rgb565::WHITE),
            drawn: Bounds::EMPTY,
            changed: full,
        }
    }

    /// The canvas size in pixels.
    pub const fn size(&self) -> Size {
        self.size
    }

    /// Fill the whole canvas with one colour.
    ///
    /// Clearing to the colour of the previous `clear` only repaints the area
    /// drawn since then, which is the common case of "clear, draw, show" in a
    /// loop.
    pub fn clear(&mut self, color: Rgb565) {
        if self.background == Some(color) {
            if let Some(drawn) = self.drawn.rectangle() {
                self.paint(drawn, color);
                self.changed.include(drawn);
            }
        } else {
            self.pixels.fill(color);
            self.changed = Bounds::of(self.bounding_box());
        }
        self.background = Some(color);
        self.drawn = Bounds::EMPTY;
    }

    /// Fill a rectangle with one colour; the part outside the canvas is
    /// ignored.
    pub fn fill(&mut self, area: Rectangle, color: Rgb565) {
        let area = area.intersection(&self.bounding_box());
        if area.is_zero_sized() {
            return;
        }
        self.paint(area, color);
        self.mark_drawn(area);
    }

    /// Set one pixel; a point outside the canvas is ignored.
    pub fn set(&mut self, point: Point, color: Rgb565) {
        if self.bounding_box().contains(point) {
            let index = self.index(point);
            self.pixels[index] = color;
            self.drawn.include_point(point);
            self.changed.include_point(point);
        }
    }

    /// Copy the pixels that changed since the last `show` to `surface`.
    ///
    /// Blocks until they have reached the panel: well under a millisecond for
    /// a small change, about 31 ms when every pixel changed.
    ///
    /// # Panics
    ///
    /// When `surface` is not the size of the canvas.
    pub fn show(&mut self, surface: &mut Surface<'_>) {
        assert_eq!(
            surface.size(),
            self.size,
            "the surface must be the size of the canvas"
        );
        let Some(candidates) = self.changed.rectangle() else {
            return;
        };
        self.changed = Bounds::EMPTY;

        // Walk the candidate rows and collect consecutive changed rows into
        // one window, as wide as the widest change among them.
        let mut window = Bounds::EMPTY;
        let mut last_changed_row = i32::MIN;
        for y in candidates.rows() {
            let Some((left, right)) = self.changed_columns(y, candidates.columns()) else {
                continue;
            };
            if !window.is_empty() && y - last_changed_row > MAX_GAP_ROWS + 1 {
                self.send(surface, window);
                window = Bounds::EMPTY;
            }
            window.include_point(Point::new(left, y));
            window.include_point(Point::new(right, y));
            last_changed_row = y;
        }
        if !window.is_empty() {
            self.send(surface, window);
        }
        self.panel_known = true;
    }

    /// Forget what the panel shows, so the next `show` sends the whole
    /// canvas.
    ///
    /// Call this after something other than this canvas drew on the same part
    /// of the panel, for example `Surface::render_scanlines` or another
    /// canvas.
    pub fn invalidate(&mut self) {
        self.panel_known = false;
        self.changed = Bounds::of(self.bounding_box());
    }

    /// Index of `point` (inside the canvas) in the pixel slices.
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

    /// Record that `area` (inside the canvas, not empty) was drawn on.
    fn mark_drawn(&mut self, area: Rectangle) {
        self.drawn.include(area);
        self.changed.include(area);
    }

    /// The first and last column of row `y`, within `columns`, where the
    /// canvas differs from the panel; `None` when the row is unchanged.
    fn changed_columns(&self, y: i32, columns: core::ops::Range<i32>) -> Option<(i32, i32)> {
        if !self.panel_known {
            return Some((columns.start, columns.end - 1));
        }
        let start = self.index(Point::new(columns.start, y));
        let end = start + columns.len();
        let drawn = &self.pixels[start..end];
        let shown = &self.shown[start..end];
        let differs = |(new, old): (&Rgb565, &Rgb565)| new != old;

        let first = drawn.iter().zip(shown).position(differs)?;
        let last = drawn.iter().zip(shown).rposition(differs)?;
        Some((columns.start + first as i32, columns.start + last as i32))
    }

    /// Send one window of the canvas and remember it as shown.
    fn send(&mut self, surface: &mut Surface<'_>, window: Bounds) {
        let Some(area) = window.rectangle() else {
            return;
        };
        let start = self.index(area.top_left);
        let mut rows = Rows {
            pixels: &self.pixels[start..],
            shown: &mut self.shown[start..],
            canvas_width: self.size.width as usize,
        };
        surface.subsurface(area).render_from(&mut rows);
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
        let canvas = self.bounding_box();
        // Collect the bounds locally: one comparison per pixel instead of
        // updating both trackers every time.
        let mut touched = Bounds::EMPTY;
        for Pixel(point, color) in pixels {
            if canvas.contains(point) {
                let index = self.index(point);
                self.pixels[index] = color;
                touched.include_point(point);
            }
        }
        if let Some(touched) = touched.rectangle() {
            self.mark_drawn(touched);
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

/// The smallest rectangle around a set of points, grown one point at a time.
///
/// Cheaper than combining `Rectangle`s: including a point is four
/// comparisons.
#[derive(Clone, Copy)]
struct Bounds {
    /// Top-left corner, inclusive.
    min: Point,
    /// Bottom-right corner, inclusive: a single point has `min == max`.
    max: Point,
}

impl Bounds {
    /// Contains nothing: `min` above and right of `max`.
    const EMPTY: Self = Self {
        min: Point::new(i32::MAX, i32::MAX),
        max: Point::new(i32::MIN, i32::MIN),
    };

    /// Exactly `area`, which must not be empty.
    fn of(area: Rectangle) -> Self {
        let mut bounds = Self::EMPTY;
        bounds.include(area);
        bounds
    }

    /// Whether no point has been included yet.
    fn is_empty(&self) -> bool {
        self.min.x > self.max.x
    }

    /// Grow the bounds to contain `point`.
    fn include_point(&mut self, point: Point) {
        self.min = self.min.component_min(point);
        self.max = self.max.component_max(point);
    }

    /// Grow the bounds to contain all of `area`. An empty `area` changes nothing.
    fn include(&mut self, area: Rectangle) {
        if let Some(bottom_right) = area.bottom_right() {
            self.include_point(area.top_left);
            self.include_point(bottom_right);
        }
    }

    /// The bounds as a rectangle, or `None` when they contain nothing.
    fn rectangle(&self) -> Option<Rectangle> {
        (!self.is_empty()).then(|| Rectangle::with_corners(self.min, self.max))
    }
}

/// The rows of one window of a canvas, converted for the display.
///
/// `pixels` and `shown` start at the window's top-left pixel; rows are
/// `canvas_width` apart. Every row sent is also copied into `shown`.
struct Rows<'a> {
    /// The application's pixels, from the window's top-left pixel to the end of
    /// the canvas.
    pixels: &'a [Rgb565],
    /// The copy of what the panel shows, aligned with `pixels`; updated as each
    /// row is sent.
    shown: &'a mut [Rgb565],
    /// Pixels per canvas row: how far to step in `pixels` to reach the next row.
    /// The window itself may be narrower.
    canvas_width: usize,
}

impl ScanlineSource for Rows<'_> {
    fn fill_row(&mut self, y: usize, row: &mut [u8]) {
        let start = y * self.canvas_width;
        let width = row.len() / BYTES_PER_PIXEL;
        let pixels = &self.pixels[start..start + width];
        for (bytes, pixel) in row.chunks_exact_mut(BYTES_PER_PIXEL).zip(pixels) {
            bytes.copy_from_slice(&RawU16::from(*pixel).into_inner().to_be_bytes());
        }
        self.shown[start..start + width].copy_from_slice(pixels);
    }
}
