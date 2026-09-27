//! The left half of the screen: the middle of the camera image.
//!
//! Camera rows go straight to the panel, without a canvas. This is the
//! demo's camera screen (`src/bin/demo/screens/camera.rs`), with a narrower
//! crop. While a row passes through, its green values are added up. The
//! whole view counts: the user fills it with their face.

use core::ops::Range;

use hack_and_hike::capabilities::{
    camera::{self, Frame},
    display::{BYTES_PER_PIXEL, ScanlineSource},
};

/// Width of the camera view on the screen, in pixels.
pub(crate) const WIDTH: usize = 160;
/// Camera columns cut off on the left. As many are cut off on the right.
const CROP_LEFT: usize = (camera::WIDTH - WIDTH) / 2;
/// The bytes of each camera row that are shown.
const SOURCE_BYTES: Range<usize> =
    CROP_LEFT * BYTES_PER_PIXEL..(CROP_LEFT + WIDTH) * BYTES_PER_PIXEL;

/// One camera frame, as rows for the left half of the screen.
pub(crate) struct CameraView<'a, 'f> {
    /// The frame being sent. It is borrowed mutably, so that `pump` can run
    /// between batches of rows.
    frame: &'a mut Frame<'f>,
    /// Sum of the green values (0 to 63 each) of all pixels so far.
    green_sum: u32,
    /// Number of pixels in `green_sum`.
    green_count: u32,
}

impl<'a, 'f> CameraView<'a, 'f> {
    /// A view of `frame`.
    pub(crate) fn new(frame: &'a mut Frame<'f>) -> Self {
        Self {
            frame,
            green_sum: 0,
            green_count: 0,
        }
    }

    /// The mean green value (0 to 63) of the view. Call it after the frame
    /// was sent. `None` when no pixel was seen.
    pub(crate) fn green_mean(&self) -> Option<f32> {
        (self.green_count > 0).then(|| self.green_sum as f32 / self.green_count as f32)
    }
}

/// The mean green value (0 to 63) of the view, without showing the frame.
/// Much faster than sending it to the display. Pumps the camera on every
/// row.
pub(crate) fn green_mean(frame: &mut Frame<'_>) -> f32 {
    let mut sum = 0;
    for y in 0..camera::HEIGHT {
        frame.pump();
        sum += green_sum(&frame.scanline(y)[SOURCE_BYTES]);
    }
    sum as f32 / (WIDTH * camera::HEIGHT) as f32
}

/// The sum of the green values of one row of big-endian RGB565 pixels.
fn green_sum(row: &[u8]) -> u32 {
    row.chunks_exact(BYTES_PER_PIXEL)
        .map(|pixel| {
            // RGB565, most significant byte first: RRRRRGGG GGGBBBBB.
            let value = u16::from_be_bytes([pixel[0], pixel[1]]);
            u32::from((value >> 5) & 0x3f)
        })
        .sum()
}

impl ScanlineSource for CameraView<'_, '_> {
    fn fill_row(&mut self, y: usize, row: &mut [u8]) {
        // `while_transferring` runs only while the CPU waits for the
        // display. With the green sum, filling rows is slower than sending
        // them, so there is no wait, and the camera ring would overflow.
        // So empty it on every row too.
        self.frame.pump();
        row.copy_from_slice(&self.frame.scanline(y)[SOURCE_BYTES]);
        self.green_sum += green_sum(row);
        self.green_count += WIDTH as u32;
    }

    fn while_transferring(&mut self) {
        self.frame.pump();
    }
}
