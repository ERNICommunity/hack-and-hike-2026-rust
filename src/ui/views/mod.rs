//! Semantic content-view composition.
//!
//! View implementations draw into the fixed content framebuffer. They can use
//! presentation geometry and application snapshots, but they never own or
//! configure LCD hardware.

mod imu;
mod text;

use embedded_graphics::{
    mono_font::{ascii::{FONT_6X10, FONT_8X13_BOLD}, MonoTextStyle},
    prelude::*,
    primitives::{PrimitiveStyleBuilder, Rectangle},
    text::{Baseline, Text},
};

use crate::{models::{ImuDisplay, ViewId}, network, theme};

use super::{
    framebuffer::{color, ContentFramebuffer},
    layout,
};

const WHITE: u16 = theme::WHITE_RGB565;
const BLACK: u16 = theme::BLACK_RGB565;
const DARK_BLUE: u16 = theme::DARK_BLUE_RGB565;
const DARK_GRAY: u16 = theme::DARK_GRAY_RGB565;
const LIGHT_GRAY: u16 = theme::LIGHT_GRAY_RGB565;

pub(crate) fn render_shell(frame: &mut ContentFramebuffer, view: ViewId) {
    match view {
        ViewId::Network => render_placeholder(frame, "NETWORK", "Peer communication"),
        ViewId::Imu | ViewId::Log => frame.clear(WHITE),
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
    frame.clear(WHITE);

    let title_style = MonoTextStyle::new(&FONT_8X13_BOLD, color(DARK_BLUE));
    let subtitle_style = MonoTextStyle::new(&FONT_6X10, color(DARK_GRAY));
    let title_x = ((layout::CONTENT_WIDTH as i32 - title.len() as i32 * 8) / 2).max(8);
    let subtitle_x = ((layout::CONTENT_WIDTH as i32 - subtitle.len() as i32 * 6) / 2).max(8);

    let _ = Text::with_baseline(title, Point::new(title_x, 92), title_style, Baseline::Top)
        .draw(frame);
    let _ = Text::with_baseline(
        subtitle,
        Point::new(subtitle_x, 114),
        subtitle_style,
        Baseline::Top,
    )
    .draw(frame);
}

fn render_microphone_shell(frame: &mut ContentFramebuffer) {
    frame.clear(WHITE);
    draw_waveform_panel(frame, 4, 4, "MIC L", layout::LEFT_WAVEFORM_Y);
    draw_waveform_panel(frame, 4, 122, "MIC R", layout::RIGHT_WAVEFORM_Y);
}

fn draw_waveform_panel(
    frame: &mut ContentFramebuffer,
    x: i32,
    y: i32,
    label: &str,
    canvas_y: usize,
) {
    let panel_style = PrimitiveStyleBuilder::new()
        .fill_color(color(BLACK))
        .stroke_color(color(DARK_GRAY))
        .stroke_width(1)
        .build();
    let panel = Rectangle::new(
        Point::new(x, y),
        Size::new((layout::CONTENT_WIDTH - 8) as u32, 114),
    );
    let _ = panel.into_styled(panel_style).draw(frame);

    let label_style = MonoTextStyle::new(&FONT_6X10, color(LIGHT_GRAY));
    let _ = Text::with_baseline(
        label,
        Point::new(x + 6, y + 4),
        label_style,
        Baseline::Top,
    )
    .draw(frame);

    frame.fill_rect(
        layout::WAVEFORM_X,
        canvas_y,
        layout::WAVEFORM_CANVAS_WIDTH,
        layout::WAVEFORM_CANVAS_HEIGHT,
        WHITE,
    );
}
