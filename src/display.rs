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

/// Rectangular LCD region in physical screen coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Region {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
}

impl Region {
    pub const fn new(x: usize, y: usize, width: usize, height: usize) -> Self {
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

/// Raw CPU0 hardware resources consumed by the display service.
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
}

pub fn init(resources: Resources, delay: &mut Delay) -> Display {
    Display {
        transport: transport::init(resources, delay),
        line_buffer: [0; WIDTH],
    }
}

impl Display {
    /// Render a rectangular region one scanline at a time.
    ///
    /// The closure receives a reusable RGB565 slice exactly `region.width`
    /// pixels wide. Each completed line is queued immediately, allowing CPU
    /// rendering of the next line to overlap the previous SPI-DMA transfer.
    /// No allocation occurs in this path.
    pub fn render_scanlines(
        &mut self,
        region: Region,
        mut render_line: impl FnMut(usize, &mut [Pixel]),
    ) {
        if region.is_empty() {
            return;
        }

        assert!(
            region.end_x() <= WIDTH && region.end_y() <= HEIGHT,
            "display region is outside the physical panel"
        );

        let x_start = region.x;
        let x_end = region.end_x();
        let transport = &mut self.transport;
        let line_buffer = &mut self.line_buffer;

        for local_y in 0..region.height {
            let pixels = &mut line_buffer[x_start..x_end];
            render_line(local_y, pixels);
            transport.queue_line(region.y + local_y, x_start..x_end, pixels);
        }

        transport.finish();
    }

    /// Blit a tightly packed row-major RGB565 buffer into a physical region.
    pub fn blit(&mut self, region: Region, pixels: &[Pixel]) {
        assert_eq!(
            pixels.len(),
            region.width * region.height,
            "display blit source length does not match region"
        );

        let width = region.width;
        self.render_scanlines(region, |local_y, destination| {
            let start = local_y * width;
            destination.copy_from_slice(&pixels[start..start + width]);
        });
    }
}
