//! Live camera preview.
//!
//! Frames go straight from the camera's buffer to the display without a
//! canvas: while a frame is being sent, the camera captures the next one.
//! Without a camera the screen says so.

use embedded_graphics::prelude::Dimensions as _;
use hack_and_hike::{
    capabilities::{
        camera::{self, Camera, Frame},
        display::{BYTES_PER_PIXEL, ScanlineSource, Surface},
    },
    ui::{Canvas, common, theme},
};

use crate::{layout, screens::Screen};

// The sensor image is wider than the content area: show its middle.
/// Width of the content area, in pixels.
const CONTENT_WIDTH: usize = layout::CONTENT_SIZE.width as usize;
/// Camera columns cut off on the left (and as many on the right).
const CROP_LEFT: usize = (camera::WIDTH - CONTENT_WIDTH) / 2;
/// The bytes of each camera row that are shown.
const SOURCE_BYTES: core::ops::Range<usize> =
    CROP_LEFT * BYTES_PER_PIXEL..(CROP_LEFT + CONTENT_WIDTH) * BYTES_PER_PIXEL;
const _: () = assert!(camera::HEIGHT == layout::CONTENT_SIZE.height as usize);
const _: () = assert!(camera::WIDTH >= CONTENT_WIDTH);

/// The camera screen.
pub(crate) struct CameraScreen {
    /// `None` when no camera answered at boot.
    camera: Option<Camera>,
    /// Whether the background (or the "no camera" message) must be drawn.
    background_dirty: bool,
}

impl CameraScreen {
    /// A camera screen for `camera`, if there is one.
    pub(crate) const fn new(camera: Option<Camera>) -> Self {
        Self {
            camera,
            background_dirty: true,
        }
    }
}

impl Screen for CameraScreen {
    fn enter(&mut self) {
        self.background_dirty = true;
    }

    fn leave(&mut self) {
        if let Some(camera) = &mut self.camera {
            camera.pause();
        }
    }

    fn present(&mut self, canvas: &mut Canvas, surface: &mut Surface<'_>) {
        let Some(camera) = &mut self.camera else {
            if self.background_dirty {
                self.background_dirty = false;
                canvas.clear(theme::WHITE);
                common::centered_text(
                    canvas,
                    canvas.bounding_box(),
                    "No camera detected",
                    common::BODY_FONT,
                    theme::DARK_GRAY,
                );
                canvas.show(surface);
            }
            return;
        };

        if self.background_dirty {
            self.background_dirty = false;
            surface.render_scanlines(|_, row| row.fill(theme::CHARCOAL));
        }
        if let Some(mut frame) = camera.begin_frame() {
            surface.render_from(&mut CenteredCrop { frame: &mut frame });
            frame.finish();
        }
    }
}

/// The middle of each camera row, as wide as the content area. While the LCD
/// DMA is busy, the camera keeps capturing the next frame.
struct CenteredCrop<'a, 'f> {
    frame: &'a mut Frame<'f>,
}

impl ScanlineSource for CenteredCrop<'_, '_> {
    fn fill_row(&mut self, y: usize, row: &mut [u8]) {
        row.copy_from_slice(&self.frame.scanline(y)[SOURCE_BYTES]);
    }

    fn while_transferring(&mut self) {
        self.frame.pump();
    }
}
