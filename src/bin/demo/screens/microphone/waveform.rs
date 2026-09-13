//! Draws one channel's waveform straight to the display.

use embedded_gui::Rect;
use hack_and_hike::{capabilities::display::Surface, ui::theme::pixel};

use super::POINTS;

pub(super) fn render(surface: &mut Surface<'_>, canvas: Rect, samples: &[i8; POINTS]) {
    let x = usize::try_from(canvas.x).unwrap_or(0);
    let y = usize::try_from(canvas.y).unwrap_or(0);
    let width = canvas.w as usize;
    let height = canvas.h as usize;
    let center_y = height as i32 / 2;
    let pixels_per_point = width / POINTS;
    let mut channel = surface.subsurface(x, y, width, height);

    channel.render_scanlines(|row, pixels| {
        let row = row as i32;
        pixels.fill(if row == center_y {
            pixel::LIGHT_GRAY
        } else {
            pixel::WHITE
        });

        // Connect neighbouring points vertically so steep slopes stay solid.
        for (point, window) in samples.windows(2).enumerate() {
            let previous = center_y - i32::from(window[0]);
            let current = center_y - i32::from(window[1]);
            if (previous.min(current)..=previous.max(current)).contains(&row) {
                let start = (point + 1) * pixels_per_point;
                pixels[start..start + pixels_per_point].fill(pixel::DARK_BLUE);
            }
        }
        if row == center_y - i32::from(samples[0]) {
            pixels[..pixels_per_point].fill(pixel::DARK_BLUE);
        }
    });
}
