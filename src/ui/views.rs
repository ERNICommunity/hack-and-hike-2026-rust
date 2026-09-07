//! Content-view composition into the fixed PSRAM framebuffer.
//!
//! This module owns presentation semantics: labels, placeholders, network text,
//! IMU graphics, and static microphone chrome. It does not know how pixels are
//! transported to the LCD.

use core::fmt::Write as _;

use arrayvec::ArrayString;
use embedded_graphics::{
    mono_font::{
        ascii::{FONT_6X10, FONT_8X13_BOLD},
        MonoTextStyle,
    },
    prelude::*,
    primitives::{PrimitiveStyle, PrimitiveStyleBuilder, Rectangle},
    text::{Baseline, Text},
};

use crate::{
    imu,
    models::{ImuDisplay, ViewId},
    network, theme,
};

use super::{
    framebuffer::{color, ContentFramebuffer},
    layout,
};

const WHITE: u16 = theme::WHITE_RGB565;
const BLACK: u16 = theme::BLACK_RGB565;
const DARK_BLUE: u16 = theme::DARK_BLUE_RGB565;
const LIGHT_BLUE: u16 = theme::LIGHT_BLUE_RGB565;
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
    let mut text = ArrayString::<768>::new();
    let status = match snapshot.status {
        network::Status::Starting => "STARTING",
        network::Status::Ready => "READY - WAITING FOR PEER",
        network::Status::PeerPresent => "PEER CONNECTED",
        network::Status::Fault => "RADIO FAULT",
    };

    let _ = writeln!(&mut text, "ESP-NOW  {}", status);
    let _ = writeln!(&mut text, "DEVICE  {}", snapshot.local_id);
    let _ = writeln!(
        &mut text,
        "CHANNEL {}   PEERS {}/{}",
        snapshot.channel,
        snapshot.peer_count,
        network::MAX_PEERS
    );
    let _ = writeln!(
        &mut text,
        "TX {}   RX {}   TXERR {}   INVALID {}",
        snapshot.tx_packets,
        snapshot.rx_packets,
        snapshot.tx_errors,
        snapshot.rx_invalid
    );
    let _ = writeln!(&mut text);

    if snapshot.peer_count == 0 {
        let _ = writeln!(&mut text, "Waiting for another Hack and Hike device...");
        let _ = writeln!(&mut text, "Flash this build to device #2.");
    } else {
        for (index, peer) in snapshot.peers.iter().filter(|peer| peer.present).enumerate() {
            let _ = writeln!(&mut text, "PEER {}  {}", index + 1, peer.device_id);
            let _ = writeln!(
                &mut text,
                "RSSI {} dBm  age {} ms  RX {}",
                peer.rssi_dbm, peer.age_ms, peer.rx_packets
            );
            let _ = writeln!(
                &mut text,
                "MAC {:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
                peer.mac[0], peer.mac[1], peer.mac[2], peer.mac[3], peer.mac[4], peer.mac[5]
            );
            let _ = writeln!(&mut text);
        }
    }

    render_text_page(frame, text.as_str());
}

pub(crate) fn render_log(frame: &mut ContentFramebuffer, text: &str) {
    render_text_page(frame, trailing_lines(text, layout::TEXT_VISIBLE_LINES));
}

pub(crate) fn render_imu(frame: &mut ContentFramebuffer, imu: &ImuDisplay) {
    frame.clear(WHITE);
    draw_imu_header(frame, imu);
    draw_attitude(frame, imu);
    draw_compass(frame, imu);
}

fn render_placeholder(frame: &mut ContentFramebuffer, title: &str, subtitle: &str) {
    frame.clear(WHITE);

    let title_style = MonoTextStyle::new(&FONT_8X13_BOLD, color(DARK_BLUE));
    let subtitle_style = MonoTextStyle::new(&FONT_6X10, color(DARK_GRAY));
    let title_x = ((layout::CONTENT_WIDTH as i32 - title.len() as i32 * 8) / 2).max(8);
    let subtitle_x = ((layout::CONTENT_WIDTH as i32 - subtitle.len() as i32 * 6) / 2).max(8);

    let _ = Text::with_baseline(
        title,
        Point::new(title_x, 92),
        title_style,
        Baseline::Top,
    )
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
    draw_waveform_panel(frame, 4, 4, "MIC L", layout::LEFT_WAVEFORM_REGION);
    draw_waveform_panel(frame, 4, 122, "MIC R", layout::RIGHT_WAVEFORM_REGION);
}

fn draw_waveform_panel(
    frame: &mut ContentFramebuffer,
    x: i32,
    y: i32,
    label: &str,
    canvas: crate::display::Region,
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
        canvas.x - layout::CONTENT_X,
        canvas.y,
        canvas.width,
        canvas.height,
        WHITE,
    );
}

fn render_text_page(frame: &mut ContentFramebuffer, text: &str) {
    frame.clear(WHITE);

    let style = MonoTextStyle::new(&FONT_6X10, color(BLACK));
    let mut y = layout::TEXT_TOP;
    for line in text.lines().take(layout::TEXT_VISIBLE_LINES) {
        let _ = Text::with_baseline(line, Point::new(4, y), style, Baseline::Top).draw(frame);
        y += layout::TEXT_LINE_HEIGHT;
    }
}

fn trailing_lines(text: &str, line_count: usize) -> &str {
    if line_count == 0 || text.is_empty() {
        return "";
    }

    let bytes = text.as_bytes();
    let mut seen = 0usize;
    for index in (0..bytes.len()).rev() {
        if bytes[index] != b'\n' {
            continue;
        }

        seen += 1;
        if seen > line_count {
            return &text[index + 1..];
        }
    }

    text
}

fn draw_imu_header(frame: &mut ContentFramebuffer, imu: &ImuDisplay) {
    let header = Rectangle::new(
        Point::new(6, 6),
        Size::new((layout::CONTENT_WIDTH - 12) as u32, 44),
    );
    let _ = header
        .into_styled(PrimitiveStyle::with_fill(color(DARK_BLUE)))
        .draw(frame);

    let small = MonoTextStyle::new(&FONT_6X10, color(LIGHT_GRAY));
    let white_small = MonoTextStyle::new(&FONT_6X10, color(WHITE));
    let value = MonoTextStyle::new(&FONT_8X13_BOLD, color(WHITE));

    let _ = Text::with_baseline(
        "IMU 9-AXIS",
        Point::new(12, 10),
        white_small,
        Baseline::Top,
    )
    .draw(frame);
    let _ = Text::with_baseline(
        status_text(imu.status),
        Point::new(12, 22),
        small,
        Baseline::Top,
    )
    .draw(frame);

    let mut mag = ArrayString::<32>::new();
    match imu.mag_status {
        imu::MagStatus::Ready => {
            let _ = write!(&mut mag, "MAG {} uT", imu.mag_field_ut);
        }
        imu::MagStatus::Learning => {
            let _ = write!(&mut mag, "MAG CAL {}%", imu.mag_calibration);
        }
        imu::MagStatus::Disturbed => mag.push_str("MAG DISTURBED"),
        imu::MagStatus::Missing => mag.push_str("MAG MISSING"),
    }
    let _ = Text::with_baseline(
        mag.as_str(),
        Point::new(12, 33),
        small,
        Baseline::Top,
    )
    .draw(frame);

    draw_header_value(frame, "ROLL", imu.roll_deg, 83, value, small);
    draw_header_value(frame, "PITCH", imu.pitch_deg, 143, value, small);
    draw_header_value(frame, "YAW", imu.yaw_deg, 207, value, small);
}

fn draw_header_value(
    frame: &mut ContentFramebuffer,
    label: &str,
    degrees: i32,
    x: i32,
    value_style: MonoTextStyle<'static, embedded_graphics::pixelcolor::Rgb565>,
    label_style: MonoTextStyle<'static, embedded_graphics::pixelcolor::Rgb565>,
) {
    let _ = Text::with_baseline(
        label,
        Point::new(x, 9),
        label_style,
        Baseline::Top,
    )
    .draw(frame);

    let mut text = ArrayString::<16>::new();
    let _ = write!(&mut text, "{} deg", degrees);
    let _ = Text::with_baseline(
        text.as_str(),
        Point::new(x, 22),
        value_style,
        Baseline::Top,
    )
    .draw(frame);
}

fn draw_attitude(frame: &mut ContentFramebuffer, imu: &ImuDisplay) {
    const X: usize = 6;
    const Y: usize = 56;
    const W: usize = layout::CONTENT_WIDTH - 12;
    const H: usize = 140;

    frame.fill_rect(X, Y, W, H, LIGHT_BLUE);

    let roll = imu.roll_deg.clamp(-45, 45);
    let pitch = imu.pitch_deg.clamp(-40, 40);
    let center_x = (W / 2) as i32;
    let center_y = (H / 2) as i32;

    for local_x in 0..W {
        let x = local_x as i32;
        let pitch_offset = pitch * 4 / 5;
        let roll_offset = roll * (x - center_x) / 300;
        let horizon = (center_y + pitch_offset + roll_offset).clamp(0, H as i32);
        frame.vline(X + local_x, Y + horizon as usize, H - horizon as usize, DARK_GRAY);
    }

    frame.hline(X, Y, W, LIGHT_GRAY);
    frame.hline(X, Y + H - 1, W, LIGHT_GRAY);
    frame.vline(X, Y, H, LIGHT_GRAY);
    frame.vline(X + W - 1, Y, H, LIGHT_GRAY);

    let center_abs_x = X + W / 2;
    let center_abs_y = Y + H / 2;
    frame.hline(center_abs_x - 36, center_abs_y, 26, WHITE);
    frame.hline(center_abs_x + 10, center_abs_y, 26, WHITE);
    frame.vline(center_abs_x, center_abs_y - 5, 11, WHITE);
    frame.hline(center_abs_x - 20, center_abs_y - 23, 40, WHITE);
    frame.hline(center_abs_x - 12, center_abs_y + 22, 24, WHITE);

    let roll_x = (center_abs_x as i32 + roll * 21 / 20 - 2)
        .clamp(X as i32, (X + W - 5) as i32) as usize;
    frame.fill_rect(roll_x, Y + 5, 5, 10, WHITE);

    let white_small = MonoTextStyle::new(&FONT_6X10, color(WHITE));
    let _ = Text::with_baseline(
        "PITCH / ROLL",
        Point::new((X + 5) as i32, (Y + 4) as i32),
        white_small,
        Baseline::Top,
    )
    .draw(frame);

    let footer = match imu.mag_status {
        imu::MagStatus::Learning => "Rotate through all axes - calibrating magnetometer",
        imu::MagStatus::Disturbed => "Magnetic disturbance - yaw correction paused",
        imu::MagStatus::Missing => "BMM150 unavailable - gyro yaw fallback",
        imu::MagStatus::Ready if imu.gyro_bias_ready => "Magnetic heading - gyro bias ready",
        imu::MagStatus::Ready => "Magnetic heading - gyro bias learning",
    };
    let _ = Text::with_baseline(
        footer,
        Point::new((X + 5) as i32, (Y + H - 13) as i32),
        white_small,
        Baseline::Top,
    )
    .draw(frame);

    if imu.read_errors > 0 || imu.mag_errors > 0 {
        let mut errors = ArrayString::<28>::new();
        let _ = write!(&mut errors, "I2C {} MAG {}", imu.read_errors, imu.mag_errors);
        let _ = Text::with_baseline(
            errors.as_str(),
            Point::new((X + W - 88) as i32, (Y + 4) as i32),
            white_small,
            Baseline::Top,
        )
        .draw(frame);
    }
}

fn draw_compass(frame: &mut ContentFramebuffer, imu: &ImuDisplay) {
    const X: usize = 6;
    const Y: usize = 202;
    const W: usize = layout::CONTENT_WIDTH - 12;
    const H: usize = 32;

    frame.fill_rect(X, Y, W, H, WHITE);
    frame.hline(X, Y, W, LIGHT_GRAY);
    frame.hline(X, Y + H - 1, W, LIGHT_GRAY);
    frame.vline(X, Y, H, LIGHT_GRAY);
    frame.vline(X + W - 1, Y, H, LIGHT_GRAY);
    frame.hline(X + 8, Y + 19, W - 16, LIGHT_GRAY);

    let center = (X + W / 2) as i32;
    let north_style = MonoTextStyle::new(&FONT_6X10, color(DARK_BLUE));
    let direction_style = MonoTextStyle::new(&FONT_6X10, color(DARK_GRAY));

    for (label, heading, style) in [
        ("N", 0, north_style),
        ("E", 90, direction_style),
        ("S", 180, direction_style),
        ("W", 270, direction_style),
    ] {
        let delta = wrap_heading_delta(heading, imu.yaw_deg);
        let label_x = center + delta * 58 / 100 - 3;
        if label_x >= X as i32 - 6 && label_x < (X + W) as i32 {
            let _ = Text::with_baseline(
                label,
                Point::new(label_x, (Y + 3) as i32),
                style,
                Baseline::Top,
            )
            .draw(frame);
        }
    }

    frame.fill_rect(X + W / 2 - 2, Y + 16, 4, 13, DARK_BLUE);
}

fn status_text(status: imu::Status) -> &'static str {
    match status {
        imu::Status::Starting => "STARTING",
        imu::Status::Running => "RUNNING",
        imu::Status::Degraded => "DEGRADED",
        imu::Status::Fault => "FAULT",
    }
}

fn wrap_heading_delta(target: i32, yaw: i32) -> i32 {
    let mut delta = target - yaw;
    while delta > 180 {
        delta -= 360;
    }
    while delta < -180 {
        delta += 360;
    }
    delta
}
