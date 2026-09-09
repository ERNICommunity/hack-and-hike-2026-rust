//! IMU attitude and compass view.
//!
//! KDL owns the three instrument regions. The horizon and compass are specialized
//! view pixels drawn inside those regions after layout, analogous to the direct
//! waveform canvas on the Microphone screen.

use core::fmt::Write as _;

use arrayvec::ArrayString;
use embedded_gui::prelude::*;

use crate::{data_plane, display::Display, imu as sensor, models::ImuDisplay};

use super::super::gui::{GuiFramebuffer, GuiSurface};
use super::common;

mod generated {
    use embedded_gui::prelude::*;
    embedded_gui::include_gui!("src/ui/views/imu/imu.kdl");
}

const NODE_CAPACITY: usize = 8;
const TEXT_CAPACITY: usize = 4;
const EVENT_CAPACITY: usize = 2;

// Fixed-point tan(angle) samples from 0..=80 degrees in five-degree steps.
// Q10 keeps the horizon projection cheap on the ESP32-S3 while making the
// displayed line follow the actual attitude angle instead of a damped linear
// approximation.
const TAN_SCALE: i32 = 1024;
const TAN_STEP_DEG: i32 = 5;
const TAN_MAX_DEG: i32 = 80;
const TAN_Q10: [i32; 17] = [
    0, 90, 181, 274, 373, 477, 591, 717, 859, 1024, 1220, 1462, 1774, 2196, 2814,
    3822, 5807,
];

type Context = GuiContext<'static, NODE_CAPACITY, TEXT_CAPACITY, EVENT_CAPACITY>;

#[derive(Clone, Copy)]
struct Geometry {
    header: Rect,
    attitude: Rect,
    compass: Rect,
}

pub(crate) struct View {
    gui: &'static mut Context,
    geometry: Geometry,
}

impl View {
    pub(crate) fn new() -> Self {
        let gui = data_plane::leaked_value_with(|| Context::new(Rect::new(0, 0, 276, 240)));
        let app = generated::ImuApp::build(gui).expect("IMU KDL exceeds embedded-gui capacities");
        Self {
            geometry: Geometry {
                header: required_rect(gui, app.widgets.header_slot, "IMU header"),
                attitude: required_rect(gui, app.widgets.attitude_slot, "IMU attitude"),
                compass: required_rect(gui, app.widgets.compass_slot, "IMU compass"),
            },
            gui,
        }
    }

    pub(crate) fn present_shell(&mut self, surface: &mut GuiSurface, display: &mut Display) {
        let geometry = self.geometry;
        surface.present_with_overlay(display, self.gui, move |frame| {
            draw_shell(frame, geometry);
        });
    }

    pub(crate) fn present(
        &mut self,
        surface: &mut GuiSurface,
        display: &mut Display,
        imu: &ImuDisplay,
    ) {
        let geometry = self.geometry;
        surface.present_with_overlay(display, self.gui, move |frame| {
            draw_header(frame, geometry.header, imu);
            draw_attitude(frame, geometry.attitude, imu);
            draw_compass(frame, geometry.compass, imu);
        });
    }
}

fn draw_shell(frame: &mut GuiFramebuffer, geometry: Geometry) {
    common::fill_rect(frame, geometry.header, common::dark_blue());
    common::draw_title(
        frame,
        "IMU",
        geometry.header.x + 6,
        geometry.header.y + 3,
        common::white(),
    );
    common::draw_body(
        frame,
        "WAITING",
        geometry.header.x + 6,
        geometry.header.y + 20,
        common::white(),
    );
    common::fill_rect(frame, geometry.attitude, common::light_blue());
    draw_border(frame, geometry.attitude);
    common::fill_rect(frame, geometry.compass, common::white());
    draw_border(frame, geometry.compass);
}

fn draw_header(frame: &mut GuiFramebuffer, area: Rect, imu: &ImuDisplay) {
    common::fill_rect(frame, area, common::dark_blue());
    common::draw_title(frame, "IMU", area.x + 6, area.y + 3, common::white());
    common::draw_body(
        frame,
        status_text(imu.status),
        area.x + 6,
        area.y + 19,
        common::white(),
    );

    let mut mag = ArrayString::<24>::new();
    match imu.mag_status {
        sensor::MagStatus::Ready => {
            let _ = write!(&mut mag, "MAG {}uT", imu.mag_field_ut);
        }
        sensor::MagStatus::Learning => {
            let _ = write!(&mut mag, "CAL {}%", imu.mag_calibration);
        }
        sensor::MagStatus::Disturbed => mag.push_str("MAG DISTURBED"),
        sensor::MagStatus::Missing => mag.push_str("MAG MISSING"),
    }
    common::draw_body(
        frame,
        mag.as_str(),
        area.x + 6,
        area.y + 35,
        common::light_gray(),
    );

    let (roll_deg, pitch_deg) = display_roll_pitch(imu);
    let first_x = area.x + 78;
    let column_width = ((area.w as i32 - 78) / 3).max(1);
    draw_header_value(frame, "ROLL", roll_deg, first_x, area.y);
    draw_header_value(
        frame,
        "PITCH",
        pitch_deg,
        first_x + column_width,
        area.y,
    );
    draw_header_value(
        frame,
        "YAW",
        imu.yaw_deg,
        first_x + column_width * 2,
        area.y,
    );
}

fn draw_header_value(frame: &mut GuiFramebuffer, label: &str, degrees: i32, x: i32, y: i32) {
    common::draw_body(frame, label, x, y + 3, common::white());
    let mut value = ArrayString::<12>::new();
    let _ = write!(&mut value, "{:+}", degrees);
    common::draw_title(frame, value.as_str(), x, y + 22, common::white());
}

fn draw_attitude(frame: &mut GuiFramebuffer, area: Rect, imu: &ImuDisplay) {
    let x0 = area.x;
    let y0 = area.y;
    let width = area.w as i32;
    let height = area.h as i32;
    common::fill_rect(frame, area, common::light_blue());

    let (display_roll, display_pitch) = display_roll_pitch(imu);
    let roll = display_roll.clamp(-TAN_MAX_DEG, TAN_MAX_DEG);
    let pitch = display_pitch.clamp(-TAN_MAX_DEG, TAN_MAX_DEG);
    let center_x = width / 2;
    let center_y = height / 2;
    let pitch_offset = project_angle(pitch, center_y);
    let roll_tangent = tangent_q10(roll);

    for local_x in 0..width {
        let roll_offset = roll_tangent * (local_x - center_x) / TAN_SCALE;
        let horizon = (center_y + pitch_offset + roll_offset).clamp(0, height);
        if horizon < height {
            common::vline(
                frame,
                x0 + local_x,
                y0 + horizon,
                (height - horizon) as u32,
                common::dark_gray(),
            );
        }
    }

    draw_border(frame, area);
    let cx = x0 + center_x;
    let cy = y0 + center_y;
    common::hline(frame, cx - 36, cy, 26, common::white());
    common::hline(frame, cx + 10, cy, 26, common::white());
    common::vline(frame, cx, cy - 5, 11, common::white());
    common::hline(frame, cx - 20, cy - 23, 40, common::white());
    common::hline(frame, cx - 12, cy + 22, 24, common::white());

    let roll_x = (cx + roll * 21 / 20 - 2).clamp(x0, x0 + width - 5);
    common::fill_box(frame, roll_x, y0 + 5, 5, 10, common::dark_blue());
    common::draw_body(frame, "PITCH / ROLL", x0 + 5, y0 + 4, common::dark_blue());

    let footer = if imu.read_errors > 0 || imu.mag_errors > 0 {
        let mut errors = ArrayString::<32>::new();
        let _ = write!(&mut errors, "I2C {}  MAG {}", imu.read_errors, imu.mag_errors);
        draw_footer(frame, area, errors.as_str());
        return;
    } else {
        match imu.mag_status {
            sensor::MagStatus::Learning => "Rotate device - calibrating mag",
            sensor::MagStatus::Disturbed => "Mag disturbance - yaw paused",
            sensor::MagStatus::Missing => "Mag missing - gyro yaw",
            sensor::MagStatus::Ready if imu.gyro_bias_ready => "Mag heading - gyro bias ready",
            sensor::MagStatus::Ready => "Mag heading - learning gyro bias",
        }
    };
    draw_footer(frame, area, footer);
}

fn draw_footer(frame: &mut GuiFramebuffer, area: Rect, text: &str) {
    common::draw_body(
        frame,
        text,
        area.x + 5,
        area.y + area.h as i32 - common::BODY_LINE_HEIGHT - 2,
        common::white(),
    );
}

fn draw_compass(frame: &mut GuiFramebuffer, area: Rect, imu: &ImuDisplay) {
    common::fill_rect(frame, area, common::white());
    draw_border(frame, area);
    common::hline(
        frame,
        area.x + 8,
        area.y + 19,
        area.w.saturating_sub(16) as u32,
        common::light_gray(),
    );

    let center = area.x + area.w as i32 / 2;
    for (label, heading, color) in [
        ("N", 0, common::dark_blue()),
        ("E", 90, common::dark_gray()),
        ("S", 180, common::dark_gray()),
        ("W", 270, common::dark_gray()),
    ] {
        let delta = wrap_heading_delta(heading, imu.yaw_deg);
        let label_x = center + delta * 58 / 100 - 3;
        if label_x >= area.x - 6 && label_x < area.x + area.w as i32 {
            common::draw_body(frame, label, label_x, area.y + 2, color);
        }
    }

    common::fill_box(frame, center - 2, area.y + 16, 4, 13, common::dark_blue());
}

/// Convert the BMI270 board axes to the CoreS3 Lite screen frame used by the
/// attitude view. Sensor pitch already matches screen roll. Sensor roll reads
/// -90 degrees when the device is standing upright with the camera above the
/// display, so add 90 degrees to make that physical pose the zero-pitch datum.
fn display_roll_pitch(imu: &ImuDisplay) -> (i32, i32) {
    (
        imu.pitch_deg,
        wrap_signed_degrees(imu.roll_deg.saturating_add(90)),
    )
}

fn wrap_signed_degrees(mut degrees: i32) -> i32 {
    while degrees > 180 {
        degrees -= 360;
    }
    while degrees < -180 {
        degrees += 360;
    }
    degrees
}

fn project_angle(degrees: i32, focal_pixels: i32) -> i32 {
    focal_pixels * tangent_q10(degrees) / TAN_SCALE
}

fn tangent_q10(degrees: i32) -> i32 {
    let clamped = degrees.clamp(-TAN_MAX_DEG, TAN_MAX_DEG);
    let (sign, magnitude) = if clamped < 0 {
        (-1, -clamped)
    } else {
        (1, clamped)
    };

    let lower_index = (magnitude / TAN_STEP_DEG) as usize;
    if lower_index >= TAN_Q10.len() - 1 {
        return sign * TAN_Q10[TAN_Q10.len() - 1];
    }

    let remainder = magnitude % TAN_STEP_DEG;
    let lower = TAN_Q10[lower_index];
    let upper = TAN_Q10[lower_index + 1];
    sign * (lower + (upper - lower) * remainder / TAN_STEP_DEG)
}

fn draw_border(frame: &mut GuiFramebuffer, area: Rect) {
    let width = area.w as u32;
    let height = area.h as u32;
    common::hline(frame, area.x, area.y, width, common::light_gray());
    common::hline(
        frame,
        area.x,
        area.y + area.h as i32 - 1,
        width,
        common::light_gray(),
    );
    common::vline(frame, area.x, area.y, height, common::light_gray());
    common::vline(
        frame,
        area.x + area.w as i32 - 1,
        area.y,
        height,
        common::light_gray(),
    );
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

fn required_rect(gui: &Context, id: WidgetId, name: &'static str) -> Rect {
    gui.absolute_rect(id).unwrap_or_else(|| panic!("{name} layout missing"))
}
