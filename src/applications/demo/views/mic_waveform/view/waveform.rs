//! Allocation-free direct renderer for the realtime microphone waveform.

use crate::{
    capabilities::display::Surface,
    ui::theme,
};

use super::Canvas;
use super::super::{POINTS, WaveformFrame};

pub(super) fn render(
    surface: &mut Surface<'_>,
    left: Canvas,
    right: Canvas,
    frame: &WaveformFrame,
) {
    render_channel(surface, left, &frame.left);
    render_channel(surface, right, &frame.right);
}

fn render_channel(surface: &mut Surface<'_>, canvas: Canvas, samples: &[i8; POINTS]) {
    let center_y = canvas.height as i32 / 2;
    let pixels_per_point = canvas.width / POINTS;
    let mut channel_surface = surface.subsurface(canvas.x, canvas.y, canvas.width, canvas.height);

    channel_surface.render_scanlines(|local_y, pixels| {
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
