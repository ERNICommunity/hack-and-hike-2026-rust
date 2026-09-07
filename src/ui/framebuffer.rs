//! Fixed PSRAM-backed content framebuffer.
//!
//! `ContentFramebuffer` is the sole large retained presentation buffer. Its
//! dimensions are fixed by the compile-time `UiDesign`; view renderers receive
//! `&mut ContentFramebuffer` and therefore cannot resize or replace its storage.

use core::convert::Infallible;

use embedded_graphics::{
    draw_target::DrawTarget,
    pixelcolor::{raw::RawU16, Rgb565},
    prelude::*,
    primitives::Rectangle,
};

use crate::data_plane;

use super::design::{self, UiColor};

const PIXELS: usize = design::CONTENT_WIDTH * design::CONTENT_HEIGHT;

pub(crate) fn color(value: UiColor) -> Rgb565 {
    Rgb565::from(RawU16::new(value.raw()))
}

pub(crate) struct ContentFramebuffer {
    pixels: data_plane::FixedPsramBuffer<u16>,
}

impl ContentFramebuffer {
    pub(crate) fn new() -> Self {
        Self {
            pixels: data_plane::FixedPsramBuffer::filled(
                PIXELS,
                design::UI.content_background.raw(),
            ),
        }
    }

    pub(crate) fn pixels(&self) -> &[u16] {
        self.pixels.as_slice()
    }

    pub(crate) fn clear(&mut self, value: UiColor) {
        self.pixels.as_mut_slice().fill(value.raw());
    }

    pub(crate) fn hline(&mut self, x: usize, y: usize, width: usize, value: UiColor) {
        if y >= design::CONTENT_HEIGHT || x >= design::CONTENT_WIDTH {
            return;
        }
        let end = (x + width).min(design::CONTENT_WIDTH);
        self.pixels.as_mut_slice()
            [y * design::CONTENT_WIDTH + x..y * design::CONTENT_WIDTH + end]
            .fill(value.raw());
    }

    pub(crate) fn vline(&mut self, x: usize, y: usize, height: usize, value: UiColor) {
        if x >= design::CONTENT_WIDTH || y >= design::CONTENT_HEIGHT {
            return;
        }
        let end = (y + height).min(design::CONTENT_HEIGHT);
        for yy in y..end {
            self.pixels.as_mut_slice()[yy * design::CONTENT_WIDTH + x] = value.raw();
        }
    }

    pub(crate) fn fill_rect(
        &mut self,
        x: usize,
        y: usize,
        width: usize,
        height: usize,
        value: UiColor,
    ) {
        if x >= design::CONTENT_WIDTH || y >= design::CONTENT_HEIGHT {
            return;
        }
        let x_end = (x + width).min(design::CONTENT_WIDTH);
        let y_end = (y + height).min(design::CONTENT_HEIGHT);
        for yy in y..y_end {
            self.pixels.as_mut_slice()[
                yy * design::CONTENT_WIDTH + x..yy * design::CONTENT_WIDTH + x_end
            ]
            .fill(value.raw());
        }
    }
}

impl OriginDimensions for ContentFramebuffer {
    fn size(&self) -> Size {
        Size::new(
            design::CONTENT_WIDTH as u32,
            design::CONTENT_HEIGHT as u32,
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
            if x < design::CONTENT_WIDTH && y < design::CONTENT_HEIGHT {
                self.pixels.as_mut_slice()[y * design::CONTENT_WIDTH + x] = color.into_storage();
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
            UiColor::from_rgb565(color.into_storage()),
        );
        Ok(())
    }
}
