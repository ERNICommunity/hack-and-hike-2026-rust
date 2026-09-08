//! Shared `embedded-gui` presentation surface.
//!
//! KDL-generated views render into one fixed PSRAM RGB565 framebuffer. The
//! framebuffer then crosses the framework's ownership-based `DisplayBackend` /
//! `DmaTransfer` boundary before reaching the existing CoreS3 SPI-DMA display
//! service. The current board transport pipelines one internal-RAM scanline at a
//! time, so a present completes synchronously; the ownership contract is already
//! the same contract required by a future non-blocking backend.

use embedded_graphics::{pixelcolor::{IntoStorage, Rgb565, RgbColor}, prelude::DrawTarget as _};
use embedded_gui::{
    DMACapableFrameBufferBackend, DisplayBackend, DisplayError, DmaTransfer,
    EndianCorrectedBuffer, EndianCorrection, FrameBuf, GuiContext, TransferError,
};

use crate::{data_plane, display::{Display, Region}};

use super::design;

pub(crate) type GuiFramebufferBackend = EndianCorrectedBuffer<'static, Rgb565>;
type GuiFramebuffer = FrameBuf<Rgb565, GuiFramebufferBackend>;

/// One reusable fixed-size content surface for KDL-generated views.
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

    /// Render a complete declarative screen and present it through the framework
    /// DMA ownership boundary.
    pub(crate) fn present<const NODES: usize, const TEXT: usize, const EVENTS: usize>(
        &mut self,
        display: &mut Display,
        gui: &mut GuiContext<'static, NODES, TEXT, EVENTS>,
    ) {
        let mut framebuffer = self
            .framebuffer
            .take()
            .expect("embedded-gui framebuffer missing");
        let _ = framebuffer.clear(Rgb565::WHITE);
        gui.render(&mut framebuffer)
            .expect("embedded-gui render failed");

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

    // `data_ptr()` is the DMA-backend contract for accessing the contiguous
    // framebuffer. The framebuffer is owned by this transfer for the complete
    // operation and its dimensions prove the slice length.
    let pixels = unsafe { core::slice::from_raw_parts(data.data_ptr(), pixel_count) };
    display.render_scanlines(design::CONTENT_REGION, |local_y, destination| {
        let start = local_y * design::CONTENT_WIDTH;
        let source = &pixels[start..start + design::CONTENT_WIDTH];
        for (dst, src) in destination.iter_mut().zip(source.iter().copied()) {
            *dst = src.into_storage();
        }
    });
}

// Keep these framework types deliberately contained at the presentation boundary.
const _: fn(DisplayError) = |_: DisplayError| {};
const _: fn(Region) = |_: Region| {};
