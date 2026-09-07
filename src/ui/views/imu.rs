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

use crate::{imu as sensor, models::ImuDisplay};

use super::super::{
    design,
    framebuffer::{color, ContentFramebuffer},
};

pub(super) fn render(frame: &mut ContentFramebuffer, imu: &ImuDisplay) {
    frame.clear(design::UI.imu.background);
    draw_header(frame, imu);
    draw_attitude(frame, imu);
    draw_compass(frame, imu);
}

fn draw_header(frame: &mut ContentFramebuffer, imu: &ImuDisplay) {
    let spec = design::UI.imu;
    let bounds = spec.header;
    let header = Rectangle::new(
        Point::new(bounds.x as i32, bounds.y as i32),
        Size::new(bounds.width as u32, bounds.height as u32),
    );
    let _ = header
        .into_styled(PrimitiveStyle::with_fill(color(spec.primary)))
        .draw(frame);

    let small = MonoTextStyle::new(&FONT_6X10, color(spec.border));
    let on_primary_small = MonoTextStyle::new(&FONT_6X10, color(spec.on_primary));
    let value = MonoTextStyle::new(&FONT_8X13_BOLD, color(spec.on_primary));

    let _ = Text::with_baseline(
        "IMU 9-AXIS",
        Point::new(bounds.x as i32 + 6, bounds.y as i32 + 4),
        on_primary_small,
        Baseline::Top,
    )
    .draw(frame);
    let _ = Text::with_baseline(
        status_text(imu.status),
        Point::new(bounds.x as i32 + 6, bounds.y as i32 + 16),
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
        Point::new(bounds.x as i32 + 6, bounds.y as i32 + 27),
        small,
        Baseline::Top,
    )
    .draw(frame);

    draw_header_value(frame, "ROLL", imu.roll_deg, spec.header_value_x[0], value, small);
    draw_header_value(frame, "PITCH", imu.pitch_deg, spec.header_value_x[1], value, small);
    draw_header_value(frame, "YAW", imu.yaw_deg, spec.header_value_x[2], value, small);
}

fn draw_header_value(
    frame: &mut ContentFramebuffer,
    label: &str,
    degrees: i32,
    x: i32,
    value_style: MonoTextStyle<'static, Rgb565>,
    label_style: MonoTextStyle<'static, Rgb565>,
) {
    let header_y = design::UI.imu.header.y as i32;
    let _ = Text::with_baseline(
        label,
        Point::new(x, header_y + 3),
        label_style,
        Baseline::Top,
    )
    .draw(frame);

    let mut text = ArrayString::<16>::new();
    let _ = write!(&mut text, "{} deg", degrees);
    let _ = Text::with_baseline(
        text.as_str(),
        Point::new(x, header_y + 16),
        value_style,
        Baseline::Top,
    )
    .draw(frame);
}

fn draw_attitude(frame: &mut ContentFramebuffer, imu: &ImuDisplay) {
    let spec = design::UI.imu;
    let area = spec.attitude;
    let x0 = area.x;
    let y0 = area.y;
    let width = area.width;
    let height = area.height;

    frame.fill_rect(x0, y0, width, height, spec.horizon_sky);

    let roll = imu.roll_deg.clamp(-45, 45);
    let pitch = imu.pitch_deg.clamp(-40, 40);
    let center_x = (width / 2) as i32;
    let center_y = (height / 2) as i32;

    for local_x in 0..width {
        let x = local_x as i32;
        let pitch_offset = pitch * 4 / 5;
        let roll_offset = roll * (x - center_x) / 300;
        let horizon = (center_y + pitch_offset + roll_offset).clamp(0, height as i32);
        frame.vline(
            x0 + local_x,
            y0 + horizon as usize,
            height - horizon as usize,
            spec.secondary,
        );
    }

    frame.hline(x0, y0, width, spec.border);
    frame.hline(x0, y0 + height - 1, width, spec.border);
    frame.vline(x0, y0, height, spec.border);
    frame.vline(x0 + width - 1, y0, height, spec.border);

    let center_abs_x = x0 + width / 2;
    let center_abs_y = y0 + height / 2;
    frame.hline(center_abs_x - 36, center_abs_y, 26, spec.on_primary);
    frame.hline(center_abs_x + 10, center_abs_y, 26, spec.on_primary);
    frame.vline(center_abs_x, center_abs_y - 5, 11, spec.on_primary);
    frame.hline(center_abs_x - 20, center_abs_y - 23, 40, spec.on_primary);
    frame.hline(center_abs_x - 12, center_abs_y + 22, 24, spec.on_primary);

    let roll_x = (center_abs_x as i32 + roll * 21 / 20 - 2)
        .clamp(x0 as i32, (x0 + width - 5) as i32) as usize;
    frame.fill_rect(roll_x, y0 + 5, 5, 10, spec.on_primary);

    let white_small = MonoTextStyle::new(&FONT_6X10, color(spec.on_primary));
    let _ = Text::with_baseline(
        "PITCH / ROLL",
        Point::new((x0 + 5) as i32, (y0 + 4) as i32),
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
        Point::new((x0 + 5) as i32, (y0 + height - 13) as i32),
        white_small,
        Baseline::Top,
    )
    .draw(frame);

    if imu.read_errors > 0 || imu.mag_errors > 0 {
        let mut errors = ArrayString::<28>::new();
        let _ = write!(&mut errors, "I2C {} MAG {}", imu.read_errors, imu.mag_errors);
        let _ = Text::with_baseline(
            errors.as_str(),
            Point::new((x0 + width - 88) as i32, (y0 + 4) as i32),
            white_small,
            Baseline::Top,
        )
        .draw(frame);
    }
}

fn draw_compass(frame: &mut ContentFramebuffer, imu: &ImuDisplay) {
    let spec = design::UI.imu;
    let area = spec.compass;
    let x0 = area.x;
    let y0 = area.y;
    let width = area.width;
    let height = area.height;

    frame.fill_rect(x0, y0, width, height, spec.background);
    frame.hline(x0, y0, width, spec.border);
    frame.hline(x0, y0 + height - 1, width, spec.border);
    frame.vline(x0, y0, height, spec.border);
    frame.vline(x0 + width - 1, y0, height, spec.border);
    frame.hline(x0 + 8, y0 + 19, width - 16, spec.border);

    let center = (x0 + width / 2) as i32;
    let north_style = MonoTextStyle::new(&FONT_6X10, color(spec.primary));
    let direction_style = MonoTextStyle::new(&FONT_6X10, color(spec.secondary));

    for (label, heading, style) in [
        ("N", 0, north_style),
        ("E", 90, direction_style),
        ("S", 180, direction_style),
        ("W", 270, direction_style),
    ] {
        let delta = wrap_heading_delta(heading, imu.yaw_deg);
        let label_x = center + delta * 58 / 100 - 3;
        if label_x >= x0 as i32 - 6 && label_x < (x0 + width) as i32 {
            let _ = Text::with_baseline(
                label,
                Point::new(label_x, (y0 + 3) as i32),
                style,
                Baseline::Top,
            )
            .draw(frame);
        }
    }

    frame.fill_rect(x0 + width / 2 - 2, y0 + 16, 4, 13, spec.primary);
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
