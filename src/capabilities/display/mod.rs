//! The 320x240 LCD.
//!
//! [`Display`] exclusively owns the panel. Applications draw through a
//! borrowed [`Surface`], which bounds every write to one rectangle of the
//! panel. There are two ways to draw on a surface:
//!
//! - [`Surface::render_scanlines`] hands the application one row of pixels
//!   at a time to fill in. Good for plain fills and patterns.
//! - [`Surface::render_from`] streams ready-made rows from a
//!   [`ScanlineSource`], such as a camera [`Frame`](super::camera::Frame).
//!
//! For text and shapes, draw into a [`Canvas`](crate::ui::Canvas) and let it
//! send only what changed.
//!
//! # Coordinates and colours
//!
//! The origin is the top-left corner of the screen, `x` grows to the right
//! and `y` downwards. Touch events use the same coordinates. Pixels are
//! [`Rgb565`]: 5 bits of red, 6 of green, 5 of blue.
//!
//! # Timing
//!
//! Drawing waits for the SPI DMA transfer to finish before it returns. A full
//! frame takes about 31 ms, a small rectangle a fraction of a millisecond, so
//! redraw only what changed.

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

/// Bytes of one RGB565 pixel on the wire, most significant byte first. A
/// [`ScanlineSource`] row holds this many bytes per pixel.
pub const BYTES_PER_PIXEL: usize = 2;

// The scanline scratch buffer lives in static RAM rather than on the main
// stack; `Display` keeps the only reference to it.
/// One row of pixels, [`WIDTH`] long. The `StaticCell` hands out its
/// `&'static mut` exactly once, in [`init`].
static LINE_BUFFER: StaticCell<[Rgb565; WIDTH]> = StaticCell::new();

/// The peripherals and pins wired to the LCD, consumed once by [`init`].
pub(crate) struct Resources {
    /// The SPI controller driving the panel.
    pub(crate) spi2: SPI2<'static>,
    /// The DMA channel that streams pixel data to SPI2.
    pub(crate) dma: DMA_CH1<'static>,
    /// SPI clock.
    pub(crate) sck: GPIO36<'static>,
    /// SPI data out.
    pub(crate) mosi: GPIO37<'static>,
    /// Data/command select of the panel controller.
    pub(crate) dc: GPIO35<'static>,
    /// Chip select of the panel controller.
    pub(crate) cs: GPIO3<'static>,
}

/// Application handle for the LCD; see the [module docs](self).
///
/// There is exactly one, from [`Board::init`](crate::Board::init). Borrow a
/// [`Surface`] from it to draw.
pub struct Display {
    /// The SPI DMA pipeline that sends commands and pixels to the panel.
    transport: transport::Transport,
    /// Scratch row for [`Surface::render_scanlines`], kept in static RAM
    /// rather than on the caller's stack.
    line_buffer: &'static mut [Rgb565; WIDTH],
}

/// Borrowed access to one rectangle of the panel.
///
/// Row indices and nested rectangles are local to the surface: row 0 is its
/// top row, whatever its position on the panel. A surface borrows the
/// [`Display`] mutably, so only one exists at a time; it gives the display
/// back when it goes out of scope.
pub struct Surface<'a> {
    /// The display, borrowed mutably so no other surface can draw meanwhile.
    display: &'a mut Display,
    /// The rectangle this surface covers, in panel coordinates.
    area: Rectangle,
}

/// Supplies ready-to-send rows of big-endian RGB565 bytes.
///
/// Implement this for anything that already holds pixels in that format, for
/// example a framebuffer or a camera frame, and draw it with
/// [`Surface::render_from`]. Rows are requested top to bottom, in batches.
pub trait ScanlineSource {
    /// Fill `row` with the bytes of scanline `y` of the surface being drawn.
    fn fill_row(&mut self, y: usize, row: &mut [u8]);

    /// Called repeatedly while the LCD is still receiving the previous rows.
    /// Use it for work that should overlap with the transfer, such as
    /// capturing the next camera frame. The default does nothing.
    fn while_transferring(&mut self) {}
}

/// Initialize the panel controller and the SPI DMA pipeline.
pub(crate) fn init(resources: Resources, delay: Delay) -> Display {
    Display {
        transport: transport::init(resources, delay),
        line_buffer: LINE_BUFFER.init_with(|| [Rgb565::new(0, 0, 0); WIDTH]),
    }
}

impl Display {
    /// Borrow the display for drawing inside `area`, in panel coordinates.
    /// [`SCREEN`] is the whole panel.
    ///
    /// # Panics
    ///
    /// When `area` does not fit the panel. The rectangles an application
    /// draws into are fixed by its layout, so that is a programming error,
    /// not a runtime condition.
    pub fn surface(&mut self, area: Rectangle) -> Surface<'_> {
        assert!(
            SCREEN.intersection(&area) == area,
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

    /// Borrow a smaller surface, `area` being local to this surface.
    ///
    /// # Panics
    ///
    /// When `area` does not fit inside this surface.
    pub fn subsurface(&mut self, area: Rectangle) -> Surface<'_> {
        let local = Rectangle::new(Point::zero(), self.area.size);
        assert!(
            local.intersection(&area) == area,
            "subsurface {area:?} does not fit its parent {local:?}"
        );
        Surface {
            display: &mut *self.display,
            area: Rectangle::new(self.area.top_left + area.top_left, area.size),
        }
    }

    /// Draw the whole surface one row at a time.
    ///
    /// `render_row` receives the row index (0 is the top row of the surface)
    /// and a slice of exactly [`width()`](Self::width) pixels to fill. It is
    /// called once per row, top to bottom.
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

    /// Draw the whole surface from rows that are already RGB565 big-endian
    /// bytes, for example a camera frame.
    ///
    /// `source` is asked for [`height()`](Self::height) rows of
    /// [`width()`](Self::width) pixels each.
    pub fn render_from(&mut self, source: &mut impl ScanlineSource) {
        self.display.transport.render(self.area, source);
    }
}

/// Adapts a per-row closure to the byte-oriented transport.
struct ComputedRows<'a, F> {
    /// The application's closure, called with the row index and a row of pixels
    /// to fill.
    render_row: F,
    /// Scratch row the closure fills, [`Surface::width`] pixels long; converted to
    /// bytes right after.
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
