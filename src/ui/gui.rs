//! Shared `embedded-gui` presentation surface for graphical applications.
//!
//! KDL-generated views render into one fixed PSRAM RGB565 framebuffer per
//! `GuiSurface`. The owning application chooses its dimensions; presentation
//! then copies the frame through a bounded display [`Surface`].

use embedded_graphics::{
    pixelcolor::{Rgb565, raw::RawU16},
    prelude::{DrawTarget as _, Point, RawData as _, RgbColor as _},
};
use embedded_gui::{EndianCorrectedBuffer, EndianCorrection, FrameBuf, GuiContext};

use crate::{
    capabilities::display::{ScanlineSource, Surface},
    support::memory::storage,
};

pub type GuiFramebufferBackend = EndianCorrectedBuffer<'static, Rgb565>;
pub type GuiFramebuffer = FrameBuf<Rgb565, GuiFramebufferBackend>;

/// One reusable fixed-size framebuffer whose dimensions are chosen by the
/// owning graphical application.
pub struct GuiSurface {
    framebuffer: GuiFramebuffer,
    width: usize,
    height: usize,
}

impl GuiSurface {
    pub fn new(width: usize, height: usize) -> Self {
        assert!(width != 0 && height != 0);
        let pixels = storage::leaked_filled_slice(width * height, Rgb565::WHITE);
        let backend = EndianCorrectedBuffer::new(pixels, EndianCorrection::ToBigEndian);
        Self {
            framebuffer: FrameBuf::new(backend, width, height),
            width,
            height,
        }
    }

    pub fn present_with_overlay<const NODES: usize, const TEXT: usize, const EVENTS: usize>(
        &mut self,
        surface: &mut Surface<'_>,
        gui: &mut GuiContext<'static, NODES, TEXT, EVENTS>,
        overlay: impl FnOnce(&mut GuiFramebuffer),
    ) {
        self.present_frame(surface, |framebuffer| {
            let _ = framebuffer.clear(Rgb565::WHITE);
            gui.render(framebuffer).expect("embedded-gui render failed");
            overlay(framebuffer);
        });
    }

    pub fn present_overlay_only(
        &mut self,
        surface: &mut Surface<'_>,
        overlay: impl FnOnce(&mut GuiFramebuffer),
    ) {
        self.present_frame(surface, overlay);
    }

    fn present_frame(&mut self, surface: &mut Surface<'_>, draw: impl FnOnce(&mut GuiFramebuffer)) {
        debug_assert_eq!(surface.width(), self.width);
        debug_assert_eq!(surface.height(), self.height);

        draw(&mut self.framebuffer);
        surface.render_from(&mut FramebufferRows {
            framebuffer: &self.framebuffer,
        });
    }
}

/// Streams a rendered framebuffer to the display row by row.
struct FramebufferRows<'a> {
    framebuffer: &'a GuiFramebuffer,
}

impl ScanlineSource for FramebufferRows<'_> {
    fn fill_row(&mut self, y: usize, row: &mut [u8]) {
        let y = i32::try_from(y).expect("framebuffer rows fit in i32");
        for (x, bytes) in (0..).zip(row.chunks_exact_mut(2)) {
            let color = self.framebuffer.get_color_at(Point::new(x, y));
            bytes.copy_from_slice(&RawU16::from(color).into_inner().to_be_bytes());
        }
    }
}
