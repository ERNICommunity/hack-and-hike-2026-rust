//! Display capability.
//!
//! [`Display`] exclusively owns the LCD transport. Applications draw through
//! a borrowed [`Surface`], which bounds every write to one rectangle. There
//! are two ways to draw:
//!
//! - [`Surface::render_scanlines`] hands the application one row of pixels at
//!   a time to fill in.
//! - [`Surface::render_from`] streams ready-made rows from a
//!   [`ScanlineSource`] such as a [`Canvas`](crate::ui::Canvas) or a camera
//!   frame.
//!
//! Both wait for the SPI DMA transfer to finish before they return, so a
//! full-screen draw blocks the calling loop for a few milliseconds.

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

use crate::platform;

pub const WIDTH: usize = platform::DISPLAY_WIDTH;
pub const HEIGHT: usize = platform::DISPLAY_HEIGHT;
/// The panel size in pixels.
pub const SIZE: Size = Size::new(WIDTH as u32, HEIGHT as u32);
/// The whole panel, for `display.surface(SCREEN)`.
pub const SCREEN: Rectangle = Rectangle::new(Point::zero(), SIZE);

/// Bytes of one RGB565 pixel on the wire.
pub const BYTES_PER_PIXEL: usize = 2;

// The scanline scratch buffer lives in static RAM rather than on the main
// stack; `Display` keeps the only reference to it.
static LINE_BUFFER: StaticCell<[Rgb565; WIDTH]> = StaticCell::new();

/// Raw CPU0 hardware resources consumed exactly once by `init`.
pub(crate) struct Resources {
    pub(crate) spi2: SPI2<'static>,
    pub(crate) dma: DMA_CH1<'static>,
    pub(crate) sck: GPIO36<'static>,
    pub(crate) mosi: GPIO37<'static>,
    pub(crate) dc: GPIO35<'static>,
    pub(crate) cs: GPIO3<'static>,
}

/// Exclusive owner of the LCD transport.
pub struct Display {
    transport: transport::Transport,
    line_buffer: &'static mut [Rgb565; WIDTH],
}

/// Borrowed display access restricted to one rectangle.
///
/// Coordinates handed to the application are local to the surface, and nested
/// surfaces cannot escape their parent.
pub struct Surface<'a> {
    display: &'a mut Display,
    area: Rectangle,
}

/// Supplies ready-to-send rows of big-endian RGB565 bytes.
///
/// Implement this for anything that already holds pixels in that format, for
/// example a framebuffer or a camera frame.
pub trait ScanlineSource {
    /// Fill `row` with the bytes of scanline `y` of the surface being drawn.
    fn fill_row(&mut self, y: usize, row: &mut [u8]);

    /// Called repeatedly while the LCD is still receiving the previous rows.
    /// Use it for work that should overlap with the transfer, such as
    /// capturing the next camera frame. The default does nothing.
    fn while_transferring(&mut self) {}
}

pub(crate) fn init(resources: Resources, delay: Delay) -> Display {
    Display {
        transport: transport::init(resources, delay),
        line_buffer: LINE_BUFFER.init_with(|| [Rgb565::new(0, 0, 0); WIDTH]),
    }
}

impl Display {
    /// Borrow the display for drawing inside `area`, in panel coordinates.
    ///
    /// Panics when `area` does not fit the panel: the rectangles an
    /// application draws into are fixed by its layout, so that is a
    /// programming error, not a runtime condition.
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
    pub fn size(&self) -> Size {
        self.area.size
    }

    pub fn width(&self) -> usize {
        self.area.size.width as usize
    }

    pub fn height(&self) -> usize {
        self.area.size.height as usize
    }

    /// Borrow a smaller surface, `area` being local to this surface.
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
    /// `render_row` receives the row index and a slice of exactly `width()`
    /// pixels to fill.
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
    /// bytes, for example a [`Canvas`](crate::ui::Canvas) or a camera frame.
    pub fn render_from(&mut self, source: &mut impl ScanlineSource) {
        self.display.transport.render(self.area, source);
    }
}

/// Adapts a per-row closure to the byte-oriented transport.
struct ComputedRows<'a, F> {
    render_row: F,
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
