//! Allocation-free partial renderer for the realtime microphone view.
//!
//! Static microphone chrome is buffered. Only each named channel canvas is
//! submitted at audio presentation rate. Canvas divisibility by waveform point
//! count is a compile-time invariant in `ui::design`.

use crate::{
    display::Display,
    waveform::{self, WaveformFrame},
};

use super::design::{self, ContentRect};

pub(crate) fn render(display: &mut Display, frame: &WaveformFrame) {
    let microphone = design::UI.microphone;
    render_channel(display, microphone.left.canvas, &frame.left);
    render_channel(display, microphone.right.canvas, &frame.right);
}

fn render_channel(
    display: &mut Display,
    canvas: ContentRect,
    samples: &[i8; waveform::POINTS],
) {
    let style = design::UI.microphone;
    let center_y = (canvas.height() as i32) / 2;
    let pixels_per_point = canvas.width() / waveform::POINTS;

    display.render_scanlines(canvas.screen_region(), |local_y, pixels| {
        pixels.fill(style.canvas_background.raw());

        if local_y as i32 == center_y {
            pixels.fill(style.grid.raw());
        }

        for point in 0..waveform::POINTS {
            let current_y = center_y - i32::from(samples[point]);
            let previous_y = if point == 0 {
                current_y
            } else {
                center_y - i32::from(samples[point - 1])
            };

            if (local_y as i32) >= current_y.min(previous_y)
                && (local_y as i32) <= current_y.max(previous_y)
            {
                let x = point * pixels_per_point;
                pixels[x..x + pixels_per_point].fill(style.trace.raw());
            }
        }
    });
}
