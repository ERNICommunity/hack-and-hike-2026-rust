//! Shared `embedded-gui` presentation surface owned by the application shell.
//!
//! KDL-generated views render into one fixed PSRAM RGB565 framebuffer. Every
//! application shares this same allocation; presentation then copies the frame
//! through the bounded content [`Surface`] supplied by the shell.

use embedded_graphics::{pixelcolor::Rgb565, prelude::DrawTarget as _, prelude::RgbColor as _};
use embedded_gui::{
    DMACapableFrameBufferBackend, DisplayBackend, DmaTransfer, EndianCorrectedBuffer,
    EndianCorrection, FrameBuf, GuiContext, TransferError,
};

use crate::{capabilities::display::Surface, support::memory::storage};

use super::design;

pub(crate) type GuiFramebufferBackend = EndianCorrectedBuffer<'static, Rgb565>;
pub(crate) type GuiFramebuffer = FrameBuf<Rgb565, GuiFramebufferBackend>;

const _: () = assert!(core::mem::size_of::<Rgb565>() == 2);

/// One reusable fixed-size content surface for every KDL-backed application.
pub(crate) struct GuiSurface {
    framebuffer: Option<GuiFramebuffer>,
}

impl GuiSurface {
    pub(crate) fn new() -> Self {
        let pixels = storage::leaked_filled_slice(
            design::CONTENT_WIDTH * design::CONTENT_HEIGHT,
            Rgb565::WHITE,
        );
        let backend = EndianCorrectedBuffer::new(pixels, EndianCorrection::ToBigEndian);
        Self {
            framebuffer: Some(FrameBuf::new(
                backend,
                design::CONTENT_WIDTH,
                design::CONTENT_HEIGHT,
            )),
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

    fn present_frame(
        &mut self,
        surface: &mut Surface<'_>,
        draw: impl FnOnce(&mut GuiFramebuffer),
    ) {
        debug_assert_eq!(surface.width(), design::CONTENT_WIDTH);
        debug_assert_eq!(surface.height(), design::CONTENT_HEIGHT);

        let mut framebuffer = self
            .framebuffer
            .take()
            .expect("embedded-gui framebuffer missing");
        draw(&mut framebuffer);

        let mut backend = CoreS3DisplayBackend { surface };
        let transfer = backend
            .start_dma_transfer(framebuffer)
            .expect("embedded-gui display transfer failed");
        self.framebuffer = Some(transfer.wait());
    }
}

struct CoreS3DisplayBackend<'surface, 'display> {
    surface: &'surface mut Surface<'display>,
}

struct CoreS3Transfer {
    framebuffer: Option<GuiFramebuffer>,
}

impl DmaTransfer for CoreS3Transfer {
    type Buffer = GuiFramebuffer;

    fn is_done(&self) -> bool {
        true
    }

    fn wait(mut self) -> Self::Buffer {
        self.framebuffer
            .take()
            .expect("embedded-gui transfer already consumed")
    }
}

impl DisplayBackend<{ design::CONTENT_WIDTH }, { design::CONTENT_HEIGHT }, GuiFramebufferBackend>
    for CoreS3DisplayBackend<'_, '_>
{
    type Transfer = CoreS3Transfer;

    fn start_dma_transfer(
        &mut self,
        framebuffer: GuiFramebuffer,
    ) -> Result<Self::Transfer, TransferError<GuiFramebufferBackend>> {
        present_framebuffer(self.surface, &framebuffer);
        Ok(CoreS3Transfer {
            framebuffer: Some(framebuffer),
        })
    }
}

fn present_framebuffer(surface: &mut Surface<'_>, framebuffer: &GuiFramebuffer) {
    let pixel_count = design::CONTENT_WIDTH * design::CONTENT_HEIGHT;
    let byte_count = pixel_count * core::mem::size_of::<Rgb565>();
    let data = &framebuffer.data;

    // SAFETY: `EndianCorrectedBuffer` owns one contiguous array of exactly
    // `CONTENT_WIDTH * CONTENT_HEIGHT` `Rgb565` values. The compile-time size
    // assertion proves two bytes per pixel, and this borrowed byte view is read
    // only for the synchronous LCD transfer.
    let bytes = unsafe { core::slice::from_raw_parts(data.data_ptr().cast::<u8>(), byte_count) };
    surface.render_rgb565_be_bytes(bytes);
}
