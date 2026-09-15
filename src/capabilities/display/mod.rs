//! The 320x240 LCD screen.
//!
//! [`Display`] is the only owner of the panel. Applications draw through a
//! borrowed [`Surface`]. A surface limits every write to one rectangle of the
//! panel. There are two ways to draw on a surface:
//!
//! - [`Surface::render_scanlines`] gives the application one row of pixels
//!   at a time to fill. Use it for plain fills and patterns.
//! - [`Surface::render_from`] sends ready-made rows from a
//!   [`ScanlineSource`], such as a camera [`Frame`](super::camera::Frame).
//!
//! For text and shapes, draw into a [`Canvas`](crate::ui::Canvas). The canvas
//! sends only the pixels that changed.
//!
//! # Coordinates and colours
//!
//! The origin (0, 0) is the top-left corner of the screen. `x` grows to the
//! right, and `y` grows downwards. Touch events use the same coordinates.
//! Pixels are [`Rgb565`]: 5 bits of red, 6 of green, 5 of blue.
//!
//! # Timing
//!
//! Drawing runs on CPU0, inside your call. The call returns only after the
//! pixels have been sent to the panel over the SPI bus. A full screen takes
//! about 31 ms, a small rectangle less than a millisecond. So redraw only
//! what changed.

mod controller;
mod transport;

use embedded_graphics::{
    pixelcolor::{Rgb565, raw::RawU16},
    prelude::{Point, RawData as _, Size},
    primitives::Rectangle,
};
use esp_hal::{
    delay::Delay,
    peripherals::{DMA_CH1, GPIO3, GPIO35, GPIO36, GPIO37, SPI2},
};
use static_cell::StaticCell;

use crate::board;

/// Width of the panel in pixels.
pub const WIDTH: usize = board::DISPLAY_WIDTH;
/// Height of the panel in pixels.
pub const HEIGHT: usize = board::DISPLAY_HEIGHT;
/// The panel size in pixels.
pub const SIZE: Size = Size::new(WIDTH as u32, HEIGHT as u32);
/// The whole panel, for `display.surface(SCREEN)`.
pub const SCREEN: Rectangle = Rectangle::new(Point::zero(), SIZE);

/// Bytes of one RGB565 pixel as sent to the panel, most significant byte
/// first. A [`ScanlineSource`] row has this many bytes for each pixel.
pub const BYTES_PER_PIXEL: usize = 2;

// The row buffer is in static RAM, not on the main stack. `Display` holds
// the only reference to it.
/// One row of pixels, [`WIDTH`] pixels long. The `StaticCell` gives out its
/// `&'static mut` reference only once, in [`init`].
static LINE_BUFFER: StaticCell<[Rgb565; WIDTH]> = StaticCell::new();

/// The peripherals and pins connected to the LCD. [`init`] takes them once.
pub(crate) struct Resources {
    /// The SPI controller that drives the panel.
    pub(crate) spi2: SPI2<'static>,
    /// The DMA (direct memory access) channel that sends pixel data to SPI2
    /// without the CPU.
    pub(crate) dma: DMA_CH1<'static>,
    /// SPI clock (SCK).
    pub(crate) sck: GPIO36<'static>,
    /// SPI data out (MOSI, main out, sub in).
    pub(crate) mosi: GPIO37<'static>,
    /// Data/command select (DC) of the panel controller: low for a command
    /// byte, high for data bytes.
    pub(crate) dc: GPIO35<'static>,
    /// Chip select (CS) of the panel controller, active low.
    pub(crate) cs: GPIO3<'static>,
}

/// Application handle for the LCD; see the [module docs](self).
///
/// There is exactly one, from [`Board::init`](crate::Board::init). Borrow a
/// [`Surface`] from it to draw.
pub struct Display {
    /// The SPI DMA connection that sends commands and pixels to the panel.
    transport: transport::Transport,
    /// A temporary row for [`Surface::render_scanlines`]. It is in static
    /// RAM, not on the caller's stack.
    line_buffer: &'static mut [Rgb565; WIDTH],
}

/// Access to one rectangle of the panel, borrowed from the [`Display`].
///
/// Row numbers and the rectangles of subsurfaces are relative to the
/// surface: row 0 is the top row of the surface, wherever the surface is on
/// the panel. A surface borrows the `Display` mutably, so only one surface
/// can be used at a time. The display is free again when the surface goes
/// out of scope.
pub struct Surface<'a> {
    /// The display, borrowed mutably, so no other surface can draw at the
    /// same time.
    display: &'a mut Display,
    /// The rectangle this surface covers, in panel coordinates.
    area: Rectangle,
}

/// A source of pixel rows as big-endian RGB565 bytes, ready to send.
///
/// Implement it for anything that already has pixels in this format, for
/// example a frame buffer or a camera frame. Draw it with
/// [`Surface::render_from`]. Rows are requested from top to bottom, a few
/// rows at a time.
pub trait ScanlineSource {
    /// Fill `row` with the bytes of row `y` of the surface. `y` is relative
    /// to the surface: 0 is its top row. `row` has [`BYTES_PER_PIXEL`] bytes
    /// for each pixel of the surface's width.
    fn fill_row(&mut self, y: usize, row: &mut [u8]);

    /// Called again and again while the panel still receives the previous
    /// rows. Use it for work that can run during the transfer, such as
    /// capturing the next camera frame. By default, it does nothing.
    fn while_transferring(&mut self) {}
}

/// Initialize the panel controller and the SPI DMA connection.
///
/// # Panics
///
/// When the SPI setup or the panel controller setup fails, or when it is
/// called a second time.
pub(crate) fn init(resources: Resources, delay: Delay) -> Display {
    Display {
        transport: transport::init(resources, delay),
        line_buffer: LINE_BUFFER.init_with(|| [Rgb565::new(0, 0, 0); WIDTH]),
    }
}

impl Display {
    /// Borrow the display to draw inside `area`, in panel coordinates.
    /// [`SCREEN`] is the whole panel.
    ///
    /// # Panics
    ///
    /// When `area` does not fit on the panel. An empty rectangle (width or
    /// height 0) also fits on the right or bottom edge; it draws nothing. The
    /// rectangles of an application come from its layout, so a rectangle
    /// that does not fit is a programming error.
    pub fn surface(&mut self, area: Rectangle) -> Surface<'_> {
        assert!(
            fits(SCREEN, area),
            "surface {area:?} does not fit the {WIDTH}x{HEIGHT} panel"
        );
        Surface {
            display: self,
            area,
        }
    }
}

impl Surface<'_> {
    /// The size of the surface in pixels.
    pub fn size(&self) -> Size {
        self.area.size
    }

    /// The width of the surface in pixels: the length of every row.
    pub fn width(&self) -> usize {
        self.area.size.width as usize
    }

    /// The height of the surface in pixels: the number of rows.
    pub fn height(&self) -> usize {
        self.area.size.height as usize
    }

    /// Borrow a smaller surface inside this one. `area` is relative to this
    /// surface.
    ///
    /// # Panics
    ///
    /// When `area` does not fit inside this surface. An empty rectangle
    /// (width or height 0) also fits on the right or bottom edge; it draws
    /// nothing.
    pub fn subsurface(&mut self, area: Rectangle) -> Surface<'_> {
        let local = Rectangle::new(Point::zero(), self.area.size);
        assert!(
            fits(local, area),
            "subsurface {area:?} does not fit its parent {local:?}"
        );
        Surface {
            display: &mut *self.display,
            area: Rectangle::new(self.area.top_left + area.top_left, area.size),
        }
    }

    /// Draw the whole surface, one row at a time. Return when all rows have
    /// reached the panel.
    ///
    /// `render_row` gets the row number (0 is the top row of the surface) and
    /// a slice of exactly [`width()`](Self::width) pixels to fill. It is
    /// called once for each row, from top to bottom.
    ///
    /// ```ignore
    /// display.surface(SCREEN).render_scanlines(|y, row| {
    ///     row.fill(if y < HEIGHT / 2 { Rgb565::BLUE } else { Rgb565::WHITE });
    /// });
    /// ```
    pub fn render_scanlines(&mut self, render_row: impl FnMut(usize, &mut [Rgb565])) {
        let width = self.width();
        let Surface { display, area } = self;
        let mut rows = ComputedRows {
            render_row,
            pixels: &mut display.line_buffer[..width],
        };
        display.transport.render(*area, &mut rows);
    }

    /// Draw the whole surface from rows that are already big-endian RGB565
    /// bytes, for example a camera frame. Return when all rows have reached
    /// the panel.
    ///
    /// `source` is asked for [`height()`](Self::height) rows of
    /// [`width()`](Self::width) pixels each.
    pub fn render_from(&mut self, source: &mut impl ScanlineSource) {
        self.display.transport.render(self.area, source);
    }
}

/// A [`ScanlineSource`] that gets its rows from the closure of
/// [`Surface::render_scanlines`]. The closure fills [`Rgb565`] pixels, and
/// the transport needs bytes, so this type converts each row.
struct ComputedRows<'a, F> {
    /// The application's closure. It gets the row number and a row of pixels
    /// to fill.
    render_row: F,
    /// The temporary row that the closure fills, [`Surface::width`] pixels
    /// long. `fill_row` converts it to bytes right after.
    pixels: &'a mut [Rgb565],
}

impl<F: FnMut(usize, &mut [Rgb565])> ScanlineSource for ComputedRows<'_, F> {
    fn fill_row(&mut self, y: usize, row: &mut [u8]) {
        (self.render_row)(y, self.pixels);
        for (bytes, pixel) in row.chunks_exact_mut(BYTES_PER_PIXEL).zip(&*self.pixels) {
            bytes.copy_from_slice(&RawU16::from(*pixel).into_inner().to_be_bytes());
        }
    }
}

/// Whether `area` lies completely inside `outer`, edges included.
///
/// An empty rectangle (width or height 0) also fits when it starts on the
/// right or bottom edge of `outer`. It draws nothing. A test that compares
/// `area` with its intersection with `outer` would reject it, because an
/// empty rectangle has no pixels in common with `outer`.
///
/// The calculation uses `i64`, so `x + width` cannot overflow for any
/// `i32` position and `u32` size.
fn fits(outer: Rectangle, area: Rectangle) -> bool {
    let start = |rectangle: Rectangle| {
        (
            i64::from(rectangle.top_left.x),
            i64::from(rectangle.top_left.y),
        )
    };
    let end = |rectangle: Rectangle| {
        let (x, y) = start(rectangle);
        (
            x + i64::from(rectangle.size.width),
            y + i64::from(rectangle.size.height),
        )
    };
    let (outer_x, outer_y) = start(outer);
    let (outer_right, outer_bottom) = end(outer);
    let (x, y) = start(area);
    let (right, bottom) = end(area);
    x >= outer_x && y >= outer_y && right <= outer_right && bottom <= outer_bottom
}
