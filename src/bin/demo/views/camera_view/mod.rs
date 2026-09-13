//! Stock camera presentation screen.
//!
//! The camera capability owns capture and frame lifetime. This screen owns only
//! presentation-specific cropping and the concrete LCD scanline-pump view.

use hack_and_hike::{
    capabilities::{
        camera,
        display::{ScanlineSource, Surface},
    },
    ui::theme,
};

use super::super::design;

const CAMERA_CROP_PIXELS: usize = camera::WIDTH - design::CONTENT_WIDTH;
const CAMERA_CROP_LEFT: usize = CAMERA_CROP_PIXELS / 2;
const CAMERA_CROP_RIGHT: usize = CAMERA_CROP_PIXELS - CAMERA_CROP_LEFT;
const CAMERA_SOURCE_START_BYTE: usize = CAMERA_CROP_LEFT * 2;
const CAMERA_SOURCE_END_BYTE: usize = (camera::WIDTH - CAMERA_CROP_RIGHT) * 2;

const _: () = assert!(camera::HEIGHT == design::CONTENT_HEIGHT);
const _: () = assert!(camera::WIDTH >= design::CONTENT_WIDTH);
const _: () = assert!(CAMERA_CROP_PIXELS.is_multiple_of(2));
const _: () =
    assert!(CAMERA_SOURCE_END_BYTE - CAMERA_SOURCE_START_BYTE == design::CONTENT_WIDTH * 2);

struct View;

impl View {
    const fn new() -> Self {
        Self
    }

    fn present_shell(&self, surface: &mut Surface<'_>) {
        surface.render_scanlines(|_local_y, pixels| {
            pixels.fill(theme::BLACK_RGB565);
        });
    }

    fn render(&self, surface: &mut Surface<'_>, frame: &mut camera::Frame<'_>) {
        surface.render_from(&mut CenteredCrop { frame });
    }
}

/// The middle of each camera row, as wide as the content area. While the LCD
/// DMA is busy, the camera keeps capturing the next frame.
struct CenteredCrop<'a, 'f> {
    frame: &'a mut camera::Frame<'f>,
}

impl ScanlineSource for CenteredCrop<'_, '_> {
    fn fill_row(&mut self, y: usize, row: &mut [u8]) {
        row.copy_from_slice(
            &self.frame.scanline(y)[CAMERA_SOURCE_START_BYTE..CAMERA_SOURCE_END_BYTE],
        );
    }

    fn while_transferring(&mut self) {
        self.frame.pump();
    }
}

pub(crate) struct Application {
    view: View,
}

impl Application {
    pub(crate) const fn new() -> Self {
        Self { view: View::new() }
    }

    pub(crate) fn present_shell(&self, surface: &mut Surface<'_>) {
        self.view.present_shell(surface);
    }

    pub(crate) fn render(&self, surface: &mut Surface<'_>, frame: &mut camera::Frame<'_>) {
        self.view.render(surface, frame);
    }
}
