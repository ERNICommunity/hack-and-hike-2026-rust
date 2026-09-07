//! Allocation-free partial renderer for the realtime microphone view.
//!
//! The static microphone page chrome lives in the content framebuffer. Only the
//! two waveform canvases are redrawn at audio presentation rate, avoiding a
//! full 276×240 LCD transfer on every ~32 ms update.

use crate::{
    display::{Display, Region},
    theme,
    waveform::{self, WaveformFrame},
};

use super::layout;

const BACKGROUND: u16 = theme::WHITE_RGB565;
const GRID: u16 = theme::LIGHT_GRAY_RGB565;
const TRACE: u16 = theme::DARK_BLUE_RGB565;
const PIXELS_PER_POINT: usize = layout::WAVEFORM_CANVAS_WIDTH / waveform::POINTS;

const _: () = assert!(layout::WAVEFORM_CANVAS_WIDTH % waveform::POINTS == 0);

pub(crate) fn render(display: &mut Display, frame: &WaveformFrame) {
    render_channel(display, layout::LEFT_WAVEFORM_REGION, &frame.left);
    render_channel(display, layout::RIGHT_WAVEFORM_REGION, &frame.right);
}

fn render_channel(
    display: &mut Display,
    region: Region,
    samples: &[i8; waveform::POINTS],
) {
    display.render_scanlines(region, |local_y, pixels| {
        pixels.fill(BACKGROUND);

        if local_y as i32 == layout::WAVEFORM_CENTER_Y {
            pixels.fill(GRID);
        }

        for point in 0..waveform::POINTS {
            let current_y = layout::WAVEFORM_CENTER_Y - i32::from(samples[point]);
            let previous_y = if point == 0 {
                current_y
            } else {
                layout::WAVEFORM_CENTER_Y - i32::from(samples[point - 1])
            };

            if (local_y as i32) >= current_y.min(previous_y)
                && (local_y as i32) <= current_y.max(previous_y)
            {
                let x = point * PIXELS_PER_POINT;
                pixels[x..x + PIXELS_PER_POINT].fill(TRACE);
            }
        }
    });
}
