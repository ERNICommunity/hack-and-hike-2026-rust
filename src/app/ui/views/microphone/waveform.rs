//! Allocation-free direct renderer for the realtime microphone waveform.

use crate::{
    app::model::{POINTS, WaveformFrame},
    services::display::Display,
};

use super::Canvas;
use super::super::super::{design::ContentRect, theme};

pub(super) fn render(display: &mut Display, left: Canvas, right: Canvas, frame: &WaveformFrame) {
    render_channel(display, left, &frame.left);
    render_channel(display, right, &frame.right);
}

fn render_channel(display: &mut Display, canvas: Canvas, samples: &[i8; POINTS]) {
    let center_y = canvas.height as i32 / 2;
    let pixels_per_point = canvas.width / POINTS;
    let region = ContentRect::new(canvas.x, canvas.y, canvas.width, canvas.height).screen_region();

    display.render_scanlines(region, |local_y, pixels| {
        pixels.fill(theme::WHITE_RGB565);

        if local_y as i32 == center_y {
            pixels.fill(theme::LIGHT_GRAY_RGB565);
        }

        for point in 0..POINTS {
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
                pixels[x..x + pixels_per_point].fill(theme::DARK_BLUE_RGB565);
            }
        }
    });
}
