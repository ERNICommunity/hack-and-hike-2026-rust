//! Semantic content-view composition.
//!
//! View-specific code contains drawing behavior. Editable sizing and color
//! policy lives in `ui::design` as typed compile-time data.

mod imu;
mod text;

use embedded_graphics::{
    mono_font::{ascii::{FONT_6X10, FONT_8X13_BOLD}, MonoTextStyle},
    prelude::*,
    primitives::{PrimitiveStyleBuilder, Rectangle},
    text::{Baseline, Text},
};

use crate::{models::{ImuDisplay, ViewId}, network};

use super::{
    design::{self, WaveformPanelSpec},
    framebuffer::{color, ContentFramebuffer},
};

pub(crate) fn render_shell(frame: &mut ContentFramebuffer, view: ViewId) {
    match view {
        ViewId::Network => render_placeholder(frame, "NETWORK", "Peer communication"),
        ViewId::Imu | ViewId::Log => frame.clear(design::UI.content_background),
        ViewId::Microphone => render_microphone_shell(frame),
        ViewId::Sound => render_placeholder(frame, "SOUND", "Speaker output"),
    }
}

pub(crate) fn render_network(frame: &mut ContentFramebuffer, snapshot: &network::Snapshot) {
    text::render_network(frame, snapshot);
}

pub(crate) fn render_log(frame: &mut ContentFramebuffer, contents: &str) {
    text::render_log(frame, contents);
}

pub(crate) fn render_imu(frame: &mut ContentFramebuffer, display: &ImuDisplay) {
    imu::render(frame, display);
}

fn render_placeholder(frame: &mut ContentFramebuffer, title: &str, subtitle: &str) {
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

fn render_microphone_shell(frame: &mut ContentFramebuffer) {
    let microphone = design::UI.microphone;
    frame.clear(design::UI.content_background);
    draw_waveform_panel(frame, microphone.left);
    draw_waveform_panel(frame, microphone.right);
}

fn draw_waveform_panel(frame: &mut ContentFramebuffer, panel: WaveformPanelSpec) {
    let style = design::UI.microphone;
    let panel_style = PrimitiveStyleBuilder::new()
        .fill_color(color(style.panel_fill))
        .stroke_color(color(style.panel_border))
        .stroke_width(1)
        .build();
    let bounds = Rectangle::new(
        Point::new(panel.panel.x() as i32, panel.panel.y() as i32),
        Size::new(panel.panel.width() as u32, panel.panel.height() as u32),
    );
    let _ = bounds.into_styled(panel_style).draw(frame);

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
