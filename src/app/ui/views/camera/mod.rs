//! Camera presentation view.
//!
//! The camera service owns capture and frame lifetime. This module owns only
//! presentation-specific cropping and the LCD scanline-pump strategy used while
//! the Camera screen is visible.

use crate::services::{camera, display::Display};

use super::super::{design, theme};

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

pub(in crate::app::ui) struct View;

impl View {
    pub(in crate::app::ui) const fn new() -> Self {
        Self
    }

    pub(in crate::app::ui) fn present_shell(&self, display: &mut Display) {
        display.render_scanlines(design::CONTENT_REGION, |_local_y, pixels| {
            pixels.fill(theme::BLACK_RGB565);
        });
    }

    pub(in crate::app::ui) fn render(&self, display: &mut Display, frame: &mut camera::Frame<'_>) {
        // Display the frozen QVGA frame at the original full 276x240 content size
        // while using SPI-DMA wait time to drain the following sensor frame into
        // the second PSRAM buffer. The LCD therefore receives a compact burst
        // rather than being paced by live camera scanlines.
        let _ = display.render_rgb565_be_scanlines_pumped(
            design::CONTENT_REGION,
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
