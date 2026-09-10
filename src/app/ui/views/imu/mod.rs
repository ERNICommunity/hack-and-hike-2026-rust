//! IMU attitude view with a perspective world compass.
//!
//! KDL owns the header and artificial-horizon regions. Projection, horizon/grid
//! mechanics, and compass geometry remain private view-local modules so this
//! facade only coordinates the generated GUI shell and semantic IMU state.

mod attitude;
mod compass;
mod horizon;
mod projection;

use core::fmt::Write as _;

use arrayvec::ArrayString;
use embedded_graphics::pixelcolor::Rgb565;
use embedded_gui::prelude::*;

use crate::{
    app::model::ImuDisplay,
    services::{display::Display, imu as sensor},
    support::memory::storage,
};

use super::super::gui::{GuiFramebuffer, GuiSurface};
use super::common;
use attitude::Tracker as AttitudeTracker;
use projection::{DisplayAttitude, round_degrees};

mod generated {
    use embedded_gui::prelude::*;
    embedded_gui::include_gui!("src/app/ui/views/imu/imu.kdl");
}

const NODE_CAPACITY: usize = 8;
const TEXT_CAPACITY: usize = 4;
const EVENT_CAPACITY: usize = 2;
const VIEW_WIDTH: i32 = 276;
const VIEW_HEIGHT: i32 = 240;

type Context = GuiContext<'static, NODE_CAPACITY, TEXT_CAPACITY, EVENT_CAPACITY>;

#[derive(Clone, Copy)]
struct Geometry {
    header: Rect,
    attitude: Rect,
}

pub(in crate::app::ui) struct View {
    geometry: Geometry,
    attitude: AttitudeTracker,
}

impl View {
    pub(in crate::app::ui) fn new() -> Self {
        let gui = storage::leaked_value_with(|| {
            Context::new(Rect::new(0, 0, VIEW_WIDTH as u32, VIEW_HEIGHT as u32))
        });
        let app = generated::ImuApp::build(gui).expect("IMU KDL exceeds embedded-gui capacities");
        Self {
            geometry: Geometry {
                header: required_rect(gui, app.widgets.header_slot, "IMU header"),
                attitude: required_rect(gui, app.widgets.attitude_slot, "IMU attitude"),
            },
            attitude: AttitudeTracker::new(),
        }
    }

    pub(in crate::app::ui) fn present_shell(
        &mut self,
        surface: &mut GuiSurface,
        display: &mut Display,
    ) {
        let geometry = self.geometry;
        surface.present_overlay_only(display, move |frame| {
            draw_view_gutters(frame, geometry);
            draw_shell(frame, geometry);
        });
    }

    pub(in crate::app::ui) fn present(
        &mut self,
        surface: &mut GuiSurface,
        display: &mut Display,
        imu: &ImuDisplay,
    ) {
        let geometry = self.geometry;
        let attitude = self.attitude.update(imu);
        surface.present_overlay_only(display, move |frame| {
            draw_view_gutters(frame, geometry);
            draw_header(frame, geometry.header, imu, attitude);
            horizon::draw_attitude(frame, geometry.attitude, attitude);
        });
    }
}

fn draw_view_gutters(frame: &mut GuiFramebuffer, geometry: Geometry) {
    let white = common::white();
    let header_y = geometry.header.y.clamp(0, VIEW_HEIGHT);
    let header_bottom = (geometry.header.y + geometry.header.h as i32).clamp(0, VIEW_HEIGHT);
    let attitude_y = geometry.attitude.y.clamp(0, VIEW_HEIGHT);
    let attitude_bottom = (geometry.attitude.y + geometry.attitude.h as i32).clamp(0, VIEW_HEIGHT);

    fill_band(frame, 0, 0, VIEW_WIDTH, header_y, white);
    fill_band(
        frame,
        0,
        header_bottom,
        VIEW_WIDTH,
        attitude_y - header_bottom,
        white,
    );
    fill_band(
        frame,
        0,
        attitude_bottom,
        VIEW_WIDTH,
        VIEW_HEIGHT - attitude_bottom,
        white,
    );

    let header_right = (geometry.header.x + geometry.header.w as i32).clamp(0, VIEW_WIDTH);
    fill_band(
        frame,
        0,
        header_y,
        geometry.header.x.max(0),
        geometry.header.h as i32,
        white,
    );
    fill_band(
        frame,
        header_right,
        header_y,
        VIEW_WIDTH - header_right,
        geometry.header.h as i32,
        white,
    );

    let attitude_right = (geometry.attitude.x + geometry.attitude.w as i32).clamp(0, VIEW_WIDTH);
    fill_band(
        frame,
        0,
        attitude_y,
        geometry.attitude.x.max(0),
        geometry.attitude.h as i32,
        white,
    );
    fill_band(
        frame,
        attitude_right,
        attitude_y,
        VIEW_WIDTH - attitude_right,
        geometry.attitude.h as i32,
        white,
    );
}

fn fill_band(frame: &mut GuiFramebuffer, x: i32, y: i32, width: i32, height: i32, color: Rgb565) {
    if width > 0 && height > 0 {
        common::fill_box(frame, x, y, width as u32, height as u32, color);
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
}

fn draw_header(
    frame: &mut GuiFramebuffer,
    area: Rect,
    imu: &ImuDisplay,
    attitude: DisplayAttitude,
) {
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
        sensor::MagStatus::Disturbed => {
            let _ = write!(&mut mag, "DIST {}uT", imu.mag_field_ut);
        }
        sensor::MagStatus::Missing => mag.push_str("MAG MISSING"),
    }
    common::draw_body(
        frame,
        mag.as_str(),
        area.x + 6,
        area.y + 35,
        common::light_gray(),
    );

    let first_x = area.x + 78;
    let column_width = ((area.w as i32 - 78) / 3).max(1);
    draw_header_value(
        frame,
        "ROLL",
        round_degrees(attitude.roll_deg),
        first_x,
        area.y,
    );
    draw_header_value(
        frame,
        "PITCH",
        round_degrees(attitude.pitch_deg),
        first_x + column_width,
        area.y,
    );
    draw_header_value(
        frame,
        "YAW",
        attitude.yaw_deg,
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

fn required_rect(gui: &Context, id: WidgetId, name: &'static str) -> Rect {
    gui.absolute_rect(id)
        .unwrap_or_else(|| panic!("{name} layout missing"))
}
