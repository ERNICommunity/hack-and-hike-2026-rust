//! Display capability.
//!
//! `Display` exclusively owns the LCD transport. Applications render through a
//! borrowed [`Surface`], which permanently bounds every write to one [`Region`].
//! CPU1 applies semantic brightness commands over the shared runtime I2C bus.
//! Board-level power/reset sequencing still happens in firmware bootstrap.

mod brightness;
mod controller;
mod transport;

use esp_hal::{
    delay::Delay,
    peripherals::{DMA_CH1, GPIO3, GPIO35, GPIO36, GPIO37, SPI2},
};
use static_cell::StaticCell;

use crate::platform::board;

pub(crate) use brightness::{Brightness, BrightnessControl};
pub(crate) use brightness::{
    Endpoints as BrightnessEndpoints, Runtime as BrightnessRuntime,
    init_endpoints as init_brightness_endpoints, task as brightness_task,
};

pub(crate) type Pixel = u16;

pub(crate) const WIDTH: usize = board::DISPLAY_WIDTH;
pub(crate) const HEIGHT: usize = board::DISPLAY_HEIGHT;
const RGB565_BYTES_PER_PIXEL: usize = 2;
const RAW_BATCH_BYTES: usize = WIDTH * RGB565_BYTES_PER_PIXEL * transport::RAW_BATCH_LINES;

// These fixed scratch buffers are large enough to materially affect the CPU0
// bootstrap stack (~5 KiB together). Keep their storage in internal static RAM
// and let the concrete `Display` retain exclusive mutable ownership through
// `'static` references. `init_with` constructs them in place instead of creating
// another temporary array on the already-tight main stack.
static LINE_BUFFER: StaticCell<[Pixel; WIDTH]> = StaticCell::new();
static RAW_BATCH_BUFFER: StaticCell<[u8; RAW_BATCH_BYTES]> = StaticCell::new();

/// Valid rectangular region in the physical LCD coordinate space.
///
/// Fields are private and construction checks panel bounds. Applications normally
/// see a borrowed `Surface` and use coordinates local to that surface instead of
/// constructing nested physical regions themselves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Region {
    x: usize,
    y: usize,
    width: usize,
    height: usize,
}

impl Region {
    pub(crate) const fn new(x: usize, y: usize, width: usize, height: usize) -> Self {
        assert!(x <= WIDTH && width <= WIDTH - x);
        assert!(y <= HEIGHT && height <= HEIGHT - y);
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub(crate) const fn width(self) -> usize {
        self.width
    }

    pub(crate) const fn height(self) -> usize {
        self.height
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
pub(crate) struct Resources {
    pub(crate) spi2: SPI2<'static>,
    pub(crate) dma: DMA_CH1<'static>,
    pub(crate) sck: GPIO36<'static>,
    pub(crate) mosi: GPIO37<'static>,
    pub(crate) dc: GPIO35<'static>,
    pub(crate) cs: GPIO3<'static>,
}

/// Exclusive CPU0 owner of the LCD transport and reusable scanline scratch.
pub(crate) struct Display {
    transport: transport::Transport,
    line_buffer: &'static mut [Pixel; WIDTH],
    raw_batch_buffer: &'static mut [u8; RAW_BATCH_BYTES],
}

/// Borrowed display access permanently restricted to one physical region.
///
/// The raw transport is intentionally inaccessible through this type. Nested
/// surfaces are addressed relative to their parent surface and cannot escape it.
pub(crate) struct Surface<'a> {
    display: &'a mut Display,
    region: Region,
}

pub(crate) fn init(resources: Resources, delay: &mut Delay) -> Display {
    let transport = transport::init(resources, delay);
    let line_buffer = LINE_BUFFER.init_with(|| [0; WIDTH]);
    let raw_batch_buffer = RAW_BATCH_BUFFER.init_with(|| [0; RAW_BATCH_BYTES]);

    Display {
        transport,
        line_buffer,
        raw_batch_buffer,
    }
}

impl Display {
    pub(crate) fn surface(&mut self, region: Region) -> Surface<'_> {
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

        let x_start = region.x;
        let x_end = region.end_x();
        let transport = &mut self.transport;
        let line_buffer = &mut *self.line_buffer;

        transport.begin_region(x_start..x_end, region.y..region.end_y());
        for local_y in 0..region.height {
            let pixels = &mut line_buffer[x_start..x_end];
            render_line(local_y, pixels);
            transport.queue_line(pixels);
        }

        transport.finish();
    }

    fn render_rgb565_be_bytes_region(&mut self, region: Region, bytes: &[u8]) {
        if region.is_empty() {
            return;
        }

        let row_bytes = region.width * RGB565_BYTES_PER_PIXEL;
        let expected_bytes = row_bytes * region.height;
        assert_eq!(bytes.len(), expected_bytes);
        let batch_bytes = row_bytes * transport::RAW_BATCH_LINES;
        let transport = &mut self.transport;

        transport.begin_region(region.x..region.end_x(), region.y..region.end_y());
        for batch in bytes.chunks(batch_bytes) {
            transport.queue_bytes_pumped(batch, || {});
        }
        transport.finish();
    }

    fn render_rgb565_be_scanlines_pumped_region<C>(
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
        let raw_batch_buffer = &mut *self.raw_batch_buffer;
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

impl Surface<'_> {
    pub(crate) const fn region(&self) -> Region {
        self.region
    }

    pub(crate) const fn width(&self) -> usize {
        self.region.width()
    }

    pub(crate) const fn height(&self) -> usize {
        self.region.height()
    }

    /// Borrow a stricter surface using coordinates local to this surface.
    pub(crate) fn subsurface<'a>(
        &'a mut self,
        x: usize,
        y: usize,
        width: usize,
        height: usize,
    ) -> Surface<'a> {
        assert!(x <= self.region.width && width <= self.region.width - x);
        assert!(y <= self.region.height && height <= self.region.height - y);

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

    /// Render this entire surface one scanline at a time.
    pub(crate) fn render_scanlines(&mut self, render_line: impl FnMut(usize, &mut [Pixel])) {
        self.display.render_scanlines_region(self.region, render_line);
    }

    /// Stream one complete big-endian RGB565 frame into this surface.
    pub(crate) fn render_rgb565_be_bytes(&mut self, bytes: &[u8]) {
        self.display
            .render_rgb565_be_bytes_region(self.region, bytes);
    }

    /// Stream scanlines while using LCD DMA wait time to advance another producer.
    pub(crate) fn render_rgb565_be_scanlines_pumped<C>(
        &mut self,
        context: &mut C,
        render_line: impl FnMut(&mut C, usize, &mut [u8]) -> bool,
        pump: impl FnMut(&mut C),
    ) -> bool {
        self.display.render_rgb565_be_scanlines_pumped_region(
            self.region,
            context,
            render_line,
            pump,
        )
    }
}
