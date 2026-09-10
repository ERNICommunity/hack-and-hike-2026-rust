//! Shared `embedded-gui` presentation surface.
//!
//! KDL-generated views render into one fixed PSRAM RGB565 framebuffer. A semantic
//! view may then draw a view-specific overlay into that same framebuffer before
//! it crosses the framework's ownership-based `DisplayBackend` / `DmaTransfer`
//! boundary. This keeps layout declarative without forcing dense telemetry or
//! instrument pixels through generic widget abstractions.

use embedded_graphics::{
    pixelcolor::Rgb565,
    prelude::DrawTarget as _,
    prelude::RgbColor as _,
};
use embedded_gui::{
    DMACapableFrameBufferBackend, DisplayBackend, DmaTransfer, EndianCorrectedBuffer,
    EndianCorrection, FrameBuf, GuiContext, TransferError,
};

use crate::{services::display::Display, support::memory::data_plane};

use super::design;

pub(crate) type GuiFramebufferBackend = EndianCorrectedBuffer<'static, Rgb565>;
pub(crate) type GuiFramebuffer = FrameBuf<Rgb565, GuiFramebufferBackend>;

const _: () = assert!(core::mem::size_of::<Rgb565>() == 2);

/// One reusable fixed-size content surface for every KDL-backed view.
pub(crate) struct GuiSurface {
    framebuffer: Option<GuiFramebuffer>,
}

impl GuiSurface {
    pub(crate) fn new() -> Self {
        let pixels = data_plane::leaked_filled_slice(
            design::CONTENT_WIDTH * design::CONTENT_HEIGHT,
            Rgb565::WHITE,
        );
        // The ILI9342C consumes RGB565 high byte first. Keeping the backing store
        // in that same byte order means presentation can batch-copy raw bytes to
        // DMA rather than decode every framebuffer pixel back into a host u16.
        let backend = EndianCorrectedBuffer::new(pixels, EndianCorrection::ToBigEndian);
        Self {
            framebuffer: Some(FrameBuf::new(
                backend,
                design::CONTENT_WIDTH,
                design::CONTENT_HEIGHT,
            )),
        }
    }

    /// Render KDL/widget content, then let the semantic view draw any specialized
    /// pixels inside KDL-owned regions before the framebuffer is presented.
    pub(crate) fn present_with_overlay<
        const NODES: usize,
        const TEXT: usize,
        const EVENTS: usize,
    >(
        &mut self,
        display: &mut Display,
        gui: &mut GuiContext<'static, NODES, TEXT, EVENTS>,
        overlay: impl FnOnce(&mut GuiFramebuffer),
    ) {
        self.present_frame(display, |framebuffer| {
            let _ = framebuffer.clear(Rgb565::WHITE);
            gui.render(framebuffer)
                .expect("embedded-gui render failed");
            overlay(framebuffer);
        });
    }

    /// Present a semantic renderer that deliberately covers every framebuffer
    /// pixel itself. This skips both the generic full-surface clear and KDL draw
    /// pass; views using it remain responsible for overwriting the complete
    /// 276x240 content surface before the DMA transfer starts.
    pub(crate) fn present_overlay_only(
        &mut self,
        display: &mut Display,
        overlay: impl FnOnce(&mut GuiFramebuffer),
    ) {
        self.present_frame(display, overlay);
    }

    fn present_frame(
        &mut self,
        display: &mut Display,
        draw: impl FnOnce(&mut GuiFramebuffer),
    ) {
        let mut framebuffer = self
            .framebuffer
            .take()
            .expect("embedded-gui framebuffer missing");
        draw(&mut framebuffer);

        let mut backend = CoreS3DisplayBackend { display };
        let transfer = backend
            .start_dma_transfer(framebuffer)
            .expect("embedded-gui display transfer failed");
        self.framebuffer = Some(transfer.wait());
    }
}

struct CoreS3DisplayBackend<'a> {
    display: &'a mut Display,
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
    for CoreS3DisplayBackend<'_>
{
    type Transfer = CoreS3Transfer;

    fn start_dma_transfer(
        &mut self,
        framebuffer: GuiFramebuffer,
    ) -> Result<Self::Transfer, TransferError<GuiFramebufferBackend>> {
        present_framebuffer(self.display, &framebuffer);
        Ok(CoreS3Transfer {
            framebuffer: Some(framebuffer),
        })
    }
}

fn present_framebuffer(display: &mut Display, framebuffer: &GuiFramebuffer) {
    let pixel_count = design::CONTENT_WIDTH * design::CONTENT_HEIGHT;
    let byte_count = pixel_count * core::mem::size_of::<Rgb565>();
    let data = &framebuffer.data;

    // SAFETY: `EndianCorrectedBuffer` owns one contiguous array of exactly
    // `CONTENT_WIDTH * CONTENT_HEIGHT` `Rgb565` values. The compile-time size
    // assertion proves two bytes per pixel, `data_ptr()` remains valid for this
    // borrowed framebuffer, and the transfer only reads the resulting byte view.
    // The endian-correcting backend has already arranged those bytes in LCD wire
    // order, so no typed mutation or aliasing is introduced by this slice.
    let bytes = unsafe { core::slice::from_raw_parts(data.data_ptr().cast::<u8>(), byte_count) };
    display.render_rgb565_be_bytes(design::CONTENT_REGION, bytes);
}
