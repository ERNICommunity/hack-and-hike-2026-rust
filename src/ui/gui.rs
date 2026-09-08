//! Shared `embedded-gui` presentation surface.
//!
//! KDL-generated views render into one fixed PSRAM RGB565 framebuffer. A semantic
//! view may then draw a view-specific overlay into that same framebuffer before
//! it crosses the framework's ownership-based `DisplayBackend` / `DmaTransfer`
//! boundary. This keeps layout declarative without forcing dense telemetry or
//! instrument pixels through generic widget abstractions.

use embedded_graphics::{pixelcolor::{IntoStorage, Rgb565}, prelude::DrawTarget as _};
use embedded_gui::{
    DMACapableFrameBufferBackend, DisplayBackend, DmaTransfer, EndianCorrectedBuffer,
    EndianCorrection, FrameBuf, GuiContext, TransferError,
};

use crate::{data_plane, display::Display};

use super::design;

pub(crate) type GuiFramebufferBackend = EndianCorrectedBuffer<'static, Rgb565>;
pub(crate) type GuiFramebuffer = FrameBuf<Rgb565, GuiFramebufferBackend>;

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
        let backend = EndianCorrectedBuffer::new(pixels, EndianCorrection::ToLittleEndian);
        Self {
            framebuffer: Some(FrameBuf::new(
                backend,
                design::CONTENT_WIDTH,
                design::CONTENT_HEIGHT,
            )),
        }
    }

    pub(crate) fn present<const NODES: usize, const TEXT: usize, const EVENTS: usize>(
        &mut self,
        display: &mut Display,
        gui: &mut GuiContext<'static, NODES, TEXT, EVENTS>,
    ) {
        self.present_with_overlay(display, gui, |_| {});
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
        let mut framebuffer = self
            .framebuffer
            .take()
            .expect("embedded-gui framebuffer missing");
        let _ = framebuffer.clear(Rgb565::WHITE);
        gui.render(&mut framebuffer)
            .expect("embedded-gui render failed");
        overlay(&mut framebuffer);

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
    let data = &framebuffer.data;

    // The framebuffer is owned by this transfer for the complete operation and
    // its fixed dimensions prove the contiguous slice length.
    let pixels = unsafe { core::slice::from_raw_parts(data.data_ptr(), pixel_count) };
    display.render_scanlines(design::CONTENT_REGION, |local_y, destination| {
        let start = local_y * design::CONTENT_WIDTH;
        let source = &pixels[start..start + design::CONTENT_WIDTH];
        for (dst, src) in destination.iter_mut().zip(source.iter().copied()) {
            *dst = src.into_storage();
        }
    });
}
