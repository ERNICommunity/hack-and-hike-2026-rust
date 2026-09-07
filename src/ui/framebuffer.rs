//! Fixed PSRAM-backed content framebuffer.
//!
//! This type is a generic RGB565 drawing surface. View-specific rendering lives
//! in `views`; the physical LCD transport lives in `display`.

use core::{convert::Infallible};

use embedded_graphics::{
    draw_target::DrawTarget,
    pixelcolor::{raw::RawU16, Rgb565},
    prelude::*,
    primitives::Rectangle,
};

use crate::{data_plane, theme};

use super::layout;

const PIXELS: usize = layout::CONTENT_WIDTH * layout::CONTENT_HEIGHT;

pub(crate) fn color(raw: u16) -> Rgb565 {
    Rgb565::from(RawU16::new(raw))
}

pub(crate) struct ContentFramebuffer {
    pixels: data_plane::FixedPsramBuffer<u16>,
}

impl ContentFramebuffer {
    pub(crate) fn new() -> Self {
        Self {
            pixels: data_plane::FixedPsramBuffer::filled(PIXELS, theme::WHITE_RGB565),
        }
    }

    pub(crate) fn pixels(&self) -> &[u16] {
        self.pixels.as_slice()
    }

    pub(crate) fn clear(&mut self, raw: u16) {
        self.pixels.as_mut_slice().fill(raw);
    }

    pub(crate) fn hline(&mut self, x: usize, y: usize, width: usize, raw: u16) {
        if y >= layout::CONTENT_HEIGHT || x >= layout::CONTENT_WIDTH {
            return;
        }
        let end = (x + width).min(layout::CONTENT_WIDTH);
        self.pixels.as_mut_slice()
            [y * layout::CONTENT_WIDTH + x..y * layout::CONTENT_WIDTH + end]
            .fill(raw);
    }

    pub(crate) fn vline(&mut self, x: usize, y: usize, height: usize, raw: u16) {
        if x >= layout::CONTENT_WIDTH || y >= layout::CONTENT_HEIGHT {
            return;
        }
        let end = (y + height).min(layout::CONTENT_HEIGHT);
        for yy in y..end {
            self.pixels.as_mut_slice()[yy * layout::CONTENT_WIDTH + x] = raw;
        }
    }

    pub(crate) fn fill_rect(
        &mut self,
        x: usize,
        y: usize,
        width: usize,
        height: usize,
        raw: u16,
    ) {
        if x >= layout::CONTENT_WIDTH || y >= layout::CONTENT_HEIGHT {
            return;
        }
        let x_end = (x + width).min(layout::CONTENT_WIDTH);
        let y_end = (y + height).min(layout::CONTENT_HEIGHT);
        for yy in y..y_end {
            self.pixels.as_mut_slice()[
                yy * layout::CONTENT_WIDTH + x..yy * layout::CONTENT_WIDTH + x_end
            ]
            .fill(raw);
        }
    }
}

impl OriginDimensions for ContentFramebuffer {
    fn size(&self) -> Size {
        Size::new(
            layout::CONTENT_WIDTH as u32,
            layout::CONTENT_HEIGHT as u32,
        )
    }
}

impl DrawTarget for ContentFramebuffer {
    type Color = Rgb565;
    type Error = Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        for Pixel(point, color) in pixels {
            if point.x < 0 || point.y < 0 {
                continue;
            }
            let x = point.x as usize;
            let y = point.y as usize;
            if x < layout::CONTENT_WIDTH && y < layout::CONTENT_HEIGHT {
                self.pixels.as_mut_slice()[y * layout::CONTENT_WIDTH + x] = color.into_storage();
            }
        }
        Ok(())
    }

    fn fill_solid(&mut self, area: &Rectangle, color: Self::Color) -> Result<(), Self::Error> {
        let bounds = area.intersection(&self.bounding_box());
        if bounds.size == Size::zero() {
            return Ok(());
        }

        self.fill_rect(
            bounds.top_left.x as usize,
            bounds.top_left.y as usize,
            bounds.size.width as usize,
            bounds.size.height as usize,
            color.into_storage(),
        );
        Ok(())
    }
}
