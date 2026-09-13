//! Live camera preview.

use embedded_gui::Rect;
use hack_and_hike::{
    capabilities::{
        camera::{self, Camera, Frame},
        display::{ScanlineSource, Surface},
    },
    ui::{common, gui::GuiSurface, theme},
};

use crate::{layout, screens::Screen};

// The sensor image is wider than the content area: show its middle.
const CROP_PIXELS: usize = camera::WIDTH - layout::CONTENT_WIDTH;
const CROP_LEFT: usize = CROP_PIXELS / 2;
const SOURCE_START_BYTE: usize = CROP_LEFT * 2;
const SOURCE_END_BYTE: usize = SOURCE_START_BYTE + layout::CONTENT_WIDTH * 2;
const _: () = assert!(camera::HEIGHT == layout::CONTENT_HEIGHT);
const _: () = assert!(camera::WIDTH >= layout::CONTENT_WIDTH);

pub(crate) struct CameraScreen {
    camera: Option<Camera>,
    background_dirty: bool,
}

impl CameraScreen {
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

    fn present(&mut self, gui: &mut GuiSurface, surface: &mut Surface<'_>) {
        let Some(camera) = &mut self.camera else {
            if self.background_dirty {
                self.background_dirty = false;
                gui.present_custom(surface, |frame| {
                    let area = Rect::new(
                        0,
                        0,
                        layout::CONTENT_WIDTH as u32,
                        layout::CONTENT_HEIGHT as u32,
                    );
                    common::centered_text(
                        frame,
                        area,
                        "No camera detected",
                        common::BODY_FONT,
                        theme::DARK_GRAY,
                    );
                });
            }
            return;
        };

        if self.background_dirty {
            self.background_dirty = false;
            surface.render_scanlines(|_, pixels| pixels.fill(theme::pixel::CHARCOAL));
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
        row.copy_from_slice(&self.frame.scanline(y)[SOURCE_START_BYTE..SOURCE_END_BYTE]);
    }

    fn while_transferring(&mut self) {
        self.frame.pump();
    }
}
