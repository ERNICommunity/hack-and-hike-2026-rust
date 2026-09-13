//! Display capability.
//!
//! [`Display`] exclusively owns the LCD transport. Applications render through a
//! borrowed [`Surface`], which bounds every write to one [`Region`]. There are
//! two ways to draw:
//!
//! - [`Surface::render_scanlines`] hands the application one row of pixels at
//!   a time to fill in.
//! - [`Surface::render_from`] streams ready-made RGB565 rows from a
//!   [`ScanlineSource`] such as a framebuffer or a camera frame.

mod controller;
mod transport;

use esp_hal::{
    delay::Delay,
    peripherals::{DMA_CH1, GPIO3, GPIO35, GPIO36, GPIO37, SPI2},
};
use static_cell::StaticCell;

use crate::platform;

/// One RGB565 pixel in the CPU's native byte order.
pub type Pixel = u16;

pub const WIDTH: usize = platform::DISPLAY_WIDTH;
pub const HEIGHT: usize = platform::DISPLAY_HEIGHT;
const BYTES_PER_PIXEL: usize = 2;

// The scanline scratch buffer lives in static RAM rather than on the main
// stack; `Display` keeps the only reference to it.
static LINE_BUFFER: StaticCell<[Pixel; WIDTH]> = StaticCell::new();

/// A rectangle in physical LCD coordinates. Construction checks panel bounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Region {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
}

impl Region {
    /// Panics when the rectangle does not fit the panel.
    pub const fn new(x: usize, y: usize, width: usize, height: usize) -> Self {
        assert!(
            x <= WIDTH && width <= WIDTH - x,
            "region exceeds the panel width"
        );
        assert!(
            y <= HEIGHT && height <= HEIGHT - y,
            "region exceeds the panel height"
        );
        Self {
            x,
            y,
            width,
            height,
        }
    }

    const fn is_empty(self) -> bool {
        self.width == 0 || self.height == 0
    }
}

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
    line_buffer: &'static mut [Pixel; WIDTH],
}

/// Borrowed display access restricted to one region.
///
/// Coordinates handed to the application are local to the surface, and nested
/// surfaces cannot escape their parent.
pub struct Surface<'a> {
    display: &'a mut Display,
    region: Region,
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
        line_buffer: LINE_BUFFER.init_with(|| [0; WIDTH]),
    }
}

impl Display {
    /// Borrow the display for drawing inside `region`.
    pub fn surface(&mut self, region: Region) -> Surface<'_> {
        Surface {
            display: self,
            region,
        }
    }

    fn render_scanlines_region(
        &mut self,
        region: Region,
        mut render_line: impl FnMut(usize, &mut [Pixel]),
    ) {
        if region.is_empty() {
            return;
        }

        let transport = &mut self.transport;
        let pixels = &mut self.line_buffer[..region.width];

        transport.begin_region(
            region.x..region.x + region.width,
            region.y..region.y + region.height,
        );
        for y in 0..region.height {
            render_line(y, pixels);
            let buffer = transport.prepare();
            for (bytes, pixel) in buffer.chunks_exact_mut(BYTES_PER_PIXEL).zip(pixels.iter()) {
                bytes.copy_from_slice(&pixel.to_be_bytes());
            }
            transport.send(region.width * BYTES_PER_PIXEL, || {});
        }
        transport.finish(|| {});
    }

    fn render_from_region(&mut self, region: Region, source: &mut impl ScanlineSource) {
        if region.is_empty() {
            return;
        }

        let row_bytes = region.width * BYTES_PER_PIXEL;
        let transport = &mut self.transport;

        transport.begin_region(
            region.x..region.x + region.width,
            region.y..region.y + region.height,
        );
        for first_row in (0..region.height).step_by(transport::BATCH_LINES) {
            let rows = (region.height - first_row).min(transport::BATCH_LINES);
            let buffer = transport.prepare();
            for (offset, row) in buffer[..rows * row_bytes]
                .chunks_exact_mut(row_bytes)
                .enumerate()
            {
                source.fill_row(first_row + offset, row);
            }
            transport.send(rows * row_bytes, || source.while_transferring());
        }
        transport.finish(|| source.while_transferring());
    }
}

impl Surface<'_> {
    pub const fn width(&self) -> usize {
        self.region.width
    }

    pub const fn height(&self) -> usize {
        self.region.height
    }

    /// Borrow a smaller surface using coordinates local to this surface.
    pub fn subsurface(&mut self, x: usize, y: usize, width: usize, height: usize) -> Surface<'_> {
        assert!(
            x <= self.region.width && width <= self.region.width - x,
            "subsurface exceeds the parent width"
        );
        assert!(
            y <= self.region.height && height <= self.region.height - y,
            "subsurface exceeds the parent height"
        );

        Surface {
            display: &mut *self.display,
            region: Region {
                x: self.region.x + x,
                y: self.region.y + y,
                width,
                height,
            },
        }
    }

    /// Draw the whole surface one row at a time.
    ///
    /// `render_line` receives the row index and a slice of exactly `width()`
    /// pixels to fill.
    pub fn render_scanlines(&mut self, render_line: impl FnMut(usize, &mut [Pixel])) {
        self.display
            .render_scanlines_region(self.region, render_line);
    }

    /// Draw the whole surface from rows that are already RGB565 big-endian bytes.
    pub fn render_from(&mut self, source: &mut impl ScanlineSource) {
        self.display.render_from_region(self.region, source);
    }
}
