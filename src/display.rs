//! CPU0-owned physical display boundary.
//!
//! This module knows how to initialize the LCD controller transport and move
//! RGB565 pixels to it. Board-level power/reset sequencing happens in bootstrap
//! before this module is initialized. The display boundary deliberately knows
//! nothing about views, navigation, text, sensors, or presentation semantics.

mod transport;

use esp_hal::{
    delay::Delay,
    peripherals::{DMA_CH1, GPIO3, GPIO35, GPIO36, GPIO37, SPI2},
};

use crate::board;

pub type Pixel = u16;

pub const WIDTH: usize = board::DISPLAY_WIDTH;
pub const HEIGHT: usize = board::DISPLAY_HEIGHT;
const RGB565_BYTES_PER_PIXEL: usize = 2;
const RAW_BATCH_BYTES: usize = WIDTH * RGB565_BYTES_PER_PIXEL * transport::RAW_BATCH_LINES;

/// Valid rectangular region in the physical LCD coordinate space.
///
/// Fields are private and construction checks panel bounds, so every `Region`
/// value is safe to submit to `Display`. Constant UI regions therefore fail at
/// compile time if an edited design extends outside the physical panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Region {
    x: usize,
    y: usize,
    width: usize,
    height: usize,
}

impl Region {
    pub const fn new(x: usize, y: usize, width: usize, height: usize) -> Self {
        assert!(x <= WIDTH && width <= WIDTH - x);
        assert!(y <= HEIGHT && height <= HEIGHT - y);
        Self {
            x,
            y,
            width,
            height,
        }
    }

    const fn end_x(self) -> usize {
        self.x + self.width
    }

    const fn end_y(self) -> usize {
        self.y + self.height
    }

    const fn is_empty(self) -> bool {
        self.width == 0 || self.height == 0
    }
}

/// Raw CPU0 hardware resources consumed exactly once by `init`.
///
/// Moving this bundle into `Display` transfers exclusive ownership of SPI2,
/// DMA_CH1, and the LCD GPIOs to the display service.
pub struct Resources {
    pub spi2: SPI2<'static>,
    pub dma: DMA_CH1<'static>,
    pub sck: GPIO36<'static>,
    pub mosi: GPIO37<'static>,
    pub dc: GPIO35<'static>,
    pub cs: GPIO3<'static>,
}

/// Exclusive CPU0 owner of the LCD transport and reusable scanline scratch.
pub struct Display {
    transport: transport::Transport,
    line_buffer: [Pixel; WIDTH],
    raw_batch_buffer: [u8; RAW_BATCH_BYTES],
}

pub fn init(resources: Resources, delay: &mut Delay) -> Display {
    Display {
        transport: transport::init(resources, delay),
        line_buffer: [0; WIDTH],
        raw_batch_buffer: [0; RAW_BATCH_BYTES],
    }
}

impl Display {
    /// Render a valid physical region one scanline at a time.
    ///
    /// The LCD window is established once for the whole rectangle. Completed
    /// lines are then streamed consecutively through the existing ping-pong DMA
    /// buffers, allowing CPU rendering of the next line to overlap the previous
    /// SPI transfer without repeating controller commands for every scanline.
    pub fn render_scanlines(
        &mut self,
        region: Region,
        mut render_line: impl FnMut(usize, &mut [Pixel]),
    ) {
        if region.is_empty() {
            return;
        }

        let x_start = region.x;
        let x_end = region.end_x();
        let transport = &mut self.transport;
        let line_buffer = &mut self.line_buffer;

        transport.begin_region(x_start..x_end, region.y..region.end_y());
        for local_y in 0..region.height {
            let pixels = &mut line_buffer[x_start..x_end];
            render_line(local_y, pixels);
            transport.queue_line(pixels);
        }

        transport.finish();
    }

    /// Render already encoded big-endian RGB565 rows without converting them
    /// through the normal `u16` presentation scratch path.
    ///
    /// Camera frames arrive from GC0308 in exactly the byte order the ILI9342C
    /// expects on SPI. Up to four consecutive rows are therefore packed into one
    /// DMA submission. This removes the camera byte -> u16 -> byte round trip and
    /// cuts per-frame SPI DMA start/wait operations from 240 to 60 for QVGA.
    ///
    /// The callback returns `false` when its source frame becomes invalid. The
    /// remainder of the programmed LCD region is then filled with black so the
    /// controller's GRAM write stays structurally complete.
    pub fn render_rgb565_be_scanlines(
        &mut self,
        region: Region,
        mut render_line: impl FnMut(usize, &mut [u8]) -> bool,
    ) -> bool {
        if region.is_empty() {
            return true;
        }

        let row_bytes = region.width * RGB565_BYTES_PER_PIXEL;
        let transport = &mut self.transport;
        let raw_batch_buffer = &mut self.raw_batch_buffer;
        let mut valid = true;
        let mut local_y = 0usize;

        transport.begin_region(region.x..region.end_x(), region.y..region.end_y());

        while local_y < region.height {
            let lines = (region.height - local_y).min(transport::RAW_BATCH_LINES);
            let batch_bytes = lines * row_bytes;
            let batch = &mut raw_batch_buffer[..batch_bytes];

            for line in 0..lines {
                let start = line * row_bytes;
                let end = start + row_bytes;
                let row = &mut batch[start..end];

                if valid {
                    valid = render_line(local_y + line, row);
                }
                if !valid {
                    row.fill(0);
                }
            }

            transport.queue_bytes(batch);
            local_y += lines;
        }

        transport.finish();
        valid
    }

    /// Camera-specialized raw renderer that uses LCD SPI-DMA wait time to make
    /// progress on an independent context (the next camera frame in practice).
    /// `context` is passed to both callbacks sequentially so callers can borrow a
    /// single mutable camera-frame object without overlapping closure captures.
    pub fn render_rgb565_be_scanlines_pumped<C>(
        &mut self,
        region: Region,
        context: &mut C,
        mut render_line: impl FnMut(&mut C, usize, &mut [u8]) -> bool,
        mut pump: impl FnMut(&mut C),
    ) -> bool {
        if region.is_empty() {
            return true;
        }

        let row_bytes = region.width * RGB565_BYTES_PER_PIXEL;
        let transport = &mut self.transport;
        let raw_batch_buffer = &mut self.raw_batch_buffer;
        let mut valid = true;
        let mut local_y = 0usize;

        transport.begin_region(region.x..region.end_x(), region.y..region.end_y());

        while local_y < region.height {
            let lines = (region.height - local_y).min(transport::RAW_BATCH_LINES);
            let batch_bytes = lines * row_bytes;
            let batch = &mut raw_batch_buffer[..batch_bytes];

            for line in 0..lines {
                let start = line * row_bytes;
                let end = start + row_bytes;
                let row = &mut batch[start..end];

                if valid {
                    valid = render_line(context, local_y + line, row);
                }
                if !valid {
                    row.fill(0);
                }
            }

            transport.queue_bytes_pumped(batch, || pump(context));
            local_y += lines;
        }

        transport.finish_pumped(|| pump(context));
        valid
    }
}
