//! Microphone labels and realtime waveform rendering.

use embedded_graphics::{
    mono_font::{ascii::FONT_6X10, MonoTextStyle},
    prelude::*,
    text::{Baseline, Text},
};

use crate::{
    display::Display,
    waveform::{self, WaveformFrame},
};

use super::super::{
    design::{self, ContentRect, WaveformPanelSpec},
    framebuffer::{color, ContentFramebuffer},
};

pub(super) fn render_shell(frame: &mut ContentFramebuffer) {
    let microphone = design::UI.microphone;
    frame.clear(design::UI.content_background);
    draw_channel_shell(frame, microphone.left);
    draw_channel_shell(frame, microphone.right);
}

pub(super) fn render(display: &mut Display, frame: &WaveformFrame) {
    let microphone = design::UI.microphone;
    render_channel(display, microphone.left.canvas, &frame.left);
    render_channel(display, microphone.right.canvas, &frame.right);
}

fn draw_channel_shell(frame: &mut ContentFramebuffer, panel: WaveformPanelSpec) {
    let style = design::UI.microphone;
    let label_style = MonoTextStyle::new(&FONT_6X10, color(style.label));
    let _ = Text::with_baseline(
        panel.label,
        Point::new(
            (panel.panel.x() + panel.label_x_offset) as i32,
            (panel.panel.y() + panel.label_y_offset) as i32,
        ),
        label_style,
        Baseline::Top,
    )
    .draw(frame);

    frame.fill_rect(
        panel.canvas.x(),
        panel.canvas.y(),
        panel.canvas.width(),
        panel.canvas.height(),
        style.canvas_background,
    );
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
