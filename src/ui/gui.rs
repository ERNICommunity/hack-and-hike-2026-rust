//! Shared `embedded-gui` presentation surface for graphical applications.
//!
//! KDL-generated views render into one fixed PSRAM RGB565 framebuffer per
//! `GuiSurface`. The owning application chooses its dimensions; presentation
//! then copies the frame through a bounded display [`Surface`].

use embedded_graphics::{pixelcolor::Rgb565, prelude::DrawTarget as _, prelude::RgbColor as _};
use embedded_gui::{
    DMACapableFrameBufferBackend, EndianCorrectedBuffer, EndianCorrection, FrameBuf, GuiContext,
};

use crate::{capabilities::display::Surface, support::memory::storage};

pub(crate) type GuiFramebufferBackend = EndianCorrectedBuffer<'static, Rgb565>;
pub(crate) type GuiFramebuffer = FrameBuf<Rgb565, GuiFramebufferBackend>;

const _: () = assert!(core::mem::size_of::<Rgb565>() == 2);

/// One reusable fixed-size framebuffer whose dimensions are chosen by the
/// owning graphical application.
pub(crate) struct GuiSurface {
    framebuffer: Option<GuiFramebuffer>,
    width: usize,
    height: usize,
}

impl GuiSurface {
    pub(crate) fn new(width: usize, height: usize) -> Self {
        assert!(width != 0 && height != 0);
        let pixels = storage::leaked_filled_slice(width * height, Rgb565::WHITE);
        let backend = EndianCorrectedBuffer::new(pixels, EndianCorrection::ToBigEndian);
        Self {
            framebuffer: Some(FrameBuf::new(backend, width, height)),
            width,
            height,
        }
    }

    pub(crate) fn present_with_overlay<
        const NODES: usize,
        const TEXT: usize,
        const EVENTS: usize,
    >(
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

    pub(crate) fn present_overlay_only(
        &mut self,
        surface: &mut Surface<'_>,
        overlay: impl FnOnce(&mut GuiFramebuffer),
    ) {
        self.present_frame(surface, overlay);
    }

    fn present_frame(&mut self, surface: &mut Surface<'_>, draw: impl FnOnce(&mut GuiFramebuffer)) {
        debug_assert_eq!(surface.width(), self.width);
        debug_assert_eq!(surface.height(), self.height);

        let mut framebuffer = self
            .framebuffer
            .take()
            .expect("embedded-gui framebuffer missing");
        draw(&mut framebuffer);
        present_framebuffer(surface, &framebuffer, self.width, self.height);
        self.framebuffer = Some(framebuffer);
    }
}

fn present_framebuffer(
    surface: &mut Surface<'_>,
    framebuffer: &GuiFramebuffer,
    width: usize,
    height: usize,
) {
    let pixel_count = width * height;
    let byte_count = pixel_count * core::mem::size_of::<Rgb565>();
    let data = &framebuffer.data;

    // SAFETY: `EndianCorrectedBuffer` owns one contiguous array of exactly
    // `width * height` `Rgb565` values. The compile-time size assertion proves
    // two bytes per pixel, and this borrowed byte view is read only for the
    // synchronous LCD transfer.
    let bytes = unsafe { core::slice::from_raw_parts(data.data_ptr().cast::<u8>(), byte_count) };
    surface.render_rgb565_be_bytes(bytes);
}
