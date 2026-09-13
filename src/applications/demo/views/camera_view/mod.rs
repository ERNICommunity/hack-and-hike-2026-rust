//! Stock camera presentation screen.
//!
//! The camera capability owns capture and frame lifetime. This screen owns only
//! presentation-specific cropping and the concrete LCD scanline-pump view.

use crate::{
    capabilities::{camera, display::Surface},
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
const _: () = assert!(CAMERA_CROP_PIXELS % 2 == 0);
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
        debug_assert_eq!(frame.pixel_format(), camera::PixelFormat::Rgb565Be);

        // Preserve the proven direct QVGA RGB565 path. While LCD DMA transmits
        // the current batch, the callback pumps the following camera frame into
        // the capability's second PSRAM buffer.
        let _ = surface.render_rgb565_be_scanlines_pumped(
            frame,
            |frame, local_y, bytes| {
                let source = frame.scanline(local_y);
                let cropped = &source[CAMERA_SOURCE_START_BYTE..CAMERA_SOURCE_END_BYTE];
                bytes.copy_from_slice(cropped);
                true
            },
            |frame| frame.pump(),
        );
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
