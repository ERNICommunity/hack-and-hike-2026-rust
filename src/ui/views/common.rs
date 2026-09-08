//! Reusable drawing helpers shared by multiple semantic views.

use embedded_graphics::{
    mono_font::{ascii::{FONT_6X10, FONT_8X13_BOLD}, MonoTextStyle},
    prelude::*,
    text::{Baseline, Text},
};

use super::super::{
    design,
    framebuffer::{color, ContentFramebuffer},
};

pub(super) fn render_text_page(frame: &mut ContentFramebuffer, text: &str) {
    let spec = design::UI.text;
    frame.clear(spec.background);

    let style = MonoTextStyle::new(&FONT_6X10, color(spec.foreground));
    let mut y = spec.top;
    for line in text.lines().take(spec.visible_lines) {
        let _ = Text::with_baseline(
            line,
            Point::new(spec.x as i32, y as i32),
            style,
            Baseline::Top,
        )
        .draw(frame);
        y += spec.line_height;
    }
}

pub(super) fn render_placeholder(
    frame: &mut ContentFramebuffer,
    title: &str,
    subtitle: &str,
) {
    let spec = design::UI.placeholder;
    frame.clear(spec.background);

    let title_style = MonoTextStyle::new(&FONT_8X13_BOLD, color(spec.title));
    let subtitle_style = MonoTextStyle::new(&FONT_6X10, color(spec.subtitle));
    let title_x = ((design::CONTENT_WIDTH as i32 - title.len() as i32 * 8) / 2).max(8);
    let subtitle_x = ((design::CONTENT_WIDTH as i32 - subtitle.len() as i32 * 6) / 2).max(8);

    let _ = Text::with_baseline(
        title,
        Point::new(title_x, spec.title_y as i32),
        title_style,
        Baseline::Top,
    )
    .draw(frame);
    let _ = Text::with_baseline(
        subtitle,
        Point::new(subtitle_x, spec.subtitle_y as i32),
        subtitle_style,
        Baseline::Top,
    )
    .draw(frame);
}
