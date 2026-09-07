//! IMU attitude and compass view.

use core::fmt::Write as _;

use arrayvec::ArrayString;
use embedded_graphics::{
    mono_font::{ascii::{FONT_6X10, FONT_8X13_BOLD}, MonoTextStyle},
    pixelcolor::Rgb565,
    prelude::*,
    primitives::{PrimitiveStyle, Rectangle},
    text::{Baseline, Text},
};

use crate::{imu as sensor, models::ImuDisplay, theme};

use super::super::{
    framebuffer::{color, ContentFramebuffer},
    layout,
};

const WHITE: u16 = theme::WHITE_RGB565;
const DARK_BLUE: u16 = theme::DARK_BLUE_RGB565;
const LIGHT_BLUE: u16 = theme::LIGHT_BLUE_RGB565;
const DARK_GRAY: u16 = theme::DARK_GRAY_RGB565;
const LIGHT_GRAY: u16 = theme::LIGHT_GRAY_RGB565;

pub(super) fn render(frame: &mut ContentFramebuffer, imu: &ImuDisplay) {
    frame.clear(WHITE);
    draw_header(frame, imu);
    draw_attitude(frame, imu);
    draw_compass(frame, imu);
}

fn draw_header(frame: &mut ContentFramebuffer, imu: &ImuDisplay) {
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
        sensor::MagStatus::Ready => {
            let _ = write!(&mut mag, "MAG {} uT", imu.mag_field_ut);
        }
        sensor::MagStatus::Learning => {
            let _ = write!(&mut mag, "MAG CAL {}%", imu.mag_calibration);
        }
        sensor::MagStatus::Disturbed => mag.push_str("MAG DISTURBED"),
        sensor::MagStatus::Missing => mag.push_str("MAG MISSING"),
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
    value_style: MonoTextStyle<'static, Rgb565>,
    label_style: MonoTextStyle<'static, Rgb565>,
) {
    let _ = Text::with_baseline(label, Point::new(x, 9), label_style, Baseline::Top).draw(frame);

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
        frame.vline(
            X + local_x,
            Y + horizon as usize,
            H - horizon as usize,
            DARK_GRAY,
        );
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
        sensor::MagStatus::Learning => "Rotate through all axes - calibrating magnetometer",
        sensor::MagStatus::Disturbed => "Magnetic disturbance - yaw correction paused",
        sensor::MagStatus::Missing => "BMM150 unavailable - gyro yaw fallback",
        sensor::MagStatus::Ready if imu.gyro_bias_ready => "Magnetic heading - gyro bias ready",
        sensor::MagStatus::Ready => "Magnetic heading - gyro bias learning",
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

fn status_text(status: sensor::Status) -> &'static str {
    match status {
        sensor::Status::Starting => "STARTING",
        sensor::Status::Running => "RUNNING",
        sensor::Status::Degraded => "DEGRADED",
        sensor::Status::Fault => "FAULT",
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
