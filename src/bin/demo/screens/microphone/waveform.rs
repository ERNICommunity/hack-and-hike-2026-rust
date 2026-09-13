//! Draws one channel's waveform straight to the display.

use embedded_graphics::primitives::Rectangle;
use hack_and_hike::{capabilities::display::Surface, ui::theme};

use super::POINTS;

pub(super) fn render(surface: &mut Surface<'_>, area: Rectangle, samples: &[i8; POINTS]) {
    let width = area.size.width as usize;
    let center_y = area.size.height as i32 / 2;
    let pixels_per_point = width / POINTS;
    let mut channel = surface.subsurface(area);

    channel.render_scanlines(|y, row| {
        let y = y as i32;
        row.fill(if y == center_y {
            theme::LIGHT_GRAY
        } else {
            theme::WHITE
        });

        // Connect neighbouring points vertically so steep slopes stay solid.
        for (point, window) in samples.windows(2).enumerate() {
            let previous = center_y - i32::from(window[0]);
            let current = center_y - i32::from(window[1]);
            if (previous.min(current)..=previous.max(current)).contains(&y) {
                let start = (point + 1) * pixels_per_point;
                row[start..start + pixels_per_point].fill(theme::DARK_BLUE);
            }
        }
        if y == center_y - i32::from(samples[0]) {
            row[..pixels_per_point].fill(theme::DARK_BLUE);
        }
    });
}
