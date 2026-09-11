//! IMU attitude view with a perspective world compass.
//!
//! KDL owns the header and artificial-horizon regions. Projection, horizon/grid
//! mechanics, and compass geometry remain private view-local modules so this
//! facade only coordinates the generated GUI shell and semantic IMU state.

mod compass;
mod horizon;
mod projection;

use core::fmt::Write as _;

use arrayvec::ArrayString;
use embedded_graphics::pixelcolor::Rgb565;
use embedded_gui::prelude::*;

use crate::{
    app::model::ImuDisplay,
    capabilities::{display::Surface, imu as sensor},
    support::memory::storage,
};

use super::super::gui::{GuiFramebuffer, GuiSurface};
use super::common;
use projection::{DisplayAttitude, display_attitude, round_degrees};

mod generated {
    use embedded_gui::prelude::*;
    embedded_gui::include_gui!("src/app/ui/views/imu/imu.kdl");
}

const NODE_CAPACITY: usize = 8;
const TEXT_CAPACITY: usize = 4;
const EVENT_CAPACITY: usize = 2;
const VIEW_WIDTH: i32 = 276;
const VIEW_HEIGHT: i32 = 240;
const WORLDVIEW_TRACE_EVERY_FRAMES: u32 = 20;
// Dot product below cos(45deg) means the actual fused basis moved by more than
// 45 degrees between displayed frames. Unlike Euler yaw jumps, this is a real
// orientation discontinuity worth flagging.
const BASIS_JUMP_DOT: f32 = 0.70710677;

type Context = GuiContext<'static, NODE_CAPACITY, TEXT_CAPACITY, EVENT_CAPACITY>;

#[derive(Clone, Copy)]
struct Geometry {
    header: Rect,
    attitude: Rect,
}

pub(crate) struct View {
    geometry: Geometry,
    trace_frames: u32,
    previous_gravity_screen: Option<[f32; 3]>,
    previous_north_screen: Option<[f32; 3]>,
}

impl View {
    pub(crate) fn new() -> Self {
        let gui = storage::leaked_value_with(|| {
            Context::new(Rect::new(0, 0, VIEW_WIDTH as u32, VIEW_HEIGHT as u32))
        });
        let app = generated::ImuApp::build(gui).expect("IMU KDL exceeds embedded-gui capacities");
        Self {
            geometry: Geometry {
                header: required_rect(gui, app.widgets.header_slot, "IMU header"),
                attitude: required_rect(gui, app.widgets.attitude_slot, "IMU attitude"),
            },
            trace_frames: 0,
            previous_gravity_screen: None,
            previous_north_screen: None,
        }
    }

    pub(crate) fn present_shell(
        &mut self,
        gui_surface: &mut GuiSurface,
        surface: &mut Surface<'_>,
    ) {
        self.previous_gravity_screen = None;
        self.previous_north_screen = None;
        ::log::info!("WORLDVIEW-EVENT basis-history-reset");

        let geometry = self.geometry;
        gui_surface.present_overlay_only(surface, move |frame| {
            draw_view_gutters(frame, geometry);
            draw_shell(frame, geometry);
        });
    }

    pub(crate) fn present(
        &mut self,
        gui_surface: &mut GuiSurface,
        surface: &mut Surface<'_>,
        imu: &ImuDisplay,
    ) {
        let geometry = self.geometry;
        let attitude = display_attitude(imu);

        self.trace_frames = self.trace_frames.wrapping_add(1);
        let gravity_dot = self
            .previous_gravity_screen
            .map(|previous| dot3(previous, imu.gravity_screen));
        let north_dot = self
            .previous_north_screen
            .map(|previous| dot3(previous, imu.north_screen));

        if gravity_dot.map(|dot| dot < BASIS_JUMP_DOT).unwrap_or(false)
            || north_dot.map(|dot| dot < BASIS_JUMP_DOT).unwrap_or(false)
        {
            ::log::warn!(
                "WORLDVIEW-EVENT basis-jump rev={} gravity_dot={} north_dot={} g=[{},{},{}] n=[{},{},{}] sensor_rpy=[{},{},{}]",
                imu.sample_revision,
                gravity_dot.unwrap_or(1.0),
                north_dot.unwrap_or(1.0),
                imu.gravity_screen[0], imu.gravity_screen[1], imu.gravity_screen[2],
                imu.north_screen[0], imu.north_screen[1], imu.north_screen[2],
                imu.roll_deg, imu.pitch_deg, imu.yaw_deg
            );
        }
        if self.trace_frames % WORLDVIEW_TRACE_EVERY_FRAMES == 0 {
            ::log::trace!(
                "WORLDVIEW rev={} sensor_rpy=[{},{},{}] display_rpy=[{},{},{}] g=[{},{},{}] n=[{},{},{}] gdot={} ndot={} mag_status={:?} field_ut={} cal={}",
                imu.sample_revision,
                imu.roll_deg,
                imu.pitch_deg,
                imu.yaw_deg,
                attitude.roll_deg,
                attitude.pitch_deg,
                attitude.yaw_deg,
                imu.gravity_screen[0], imu.gravity_screen[1], imu.gravity_screen[2],
                imu.north_screen[0], imu.north_screen[1], imu.north_screen[2],
                gravity_dot.unwrap_or(1.0),
                north_dot.unwrap_or(1.0),
                imu.mag_status,
                imu.mag_field_ut,
                imu.mag_calibration
            );
        }
        self.previous_gravity_screen = Some(imu.gravity_screen);
        self.previous_north_screen = Some(imu.north_screen);

        gui_surface.present_overlay_only(surface, move |frame| {
            draw_view_gutters(frame, geometry);
            draw_header(frame, geometry.header, imu, attitude);
            horizon::draw_attitude(frame, geometry.attitude, attitude);
        });
    }
}

fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn draw_view_gutters(frame: &mut GuiFramebuffer, geometry: Geometry) {
    let white = common::white();
    let header_y = geometry.header.y.clamp(0, VIEW_HEIGHT);
    let header_bottom = (geometry.header.y + geometry.header.h as i32).clamp(0, VIEW_HEIGHT);
    let attitude_y = geometry.attitude.y.clamp(0, VIEW_HEIGHT);
    let attitude_bottom = (geometry.attitude.y + geometry.attitude.h as i32).clamp(0, VIEW_HEIGHT);

    fill_band(frame, 0, 0, VIEW_WIDTH, header_y, white);
    fill_band(frame, 0, header_bottom, VIEW_WIDTH, attitude_y - header_bottom, white);
    fill_band(frame, 0, attitude_bottom, VIEW_WIDTH, VIEW_HEIGHT - attitude_bottom, white);

    let header_right = (geometry.header.x + geometry.header.w as i32).clamp(0, VIEW_WIDTH);
    fill_band(frame, 0, header_y, geometry.header.x.max(0), geometry.header.h as i32, white);
    fill_band(frame, header_right, header_y, VIEW_WIDTH - header_right, geometry.header.h as i32, white);

    let attitude_right = (geometry.attitude.x + geometry.attitude.w as i32).clamp(0, VIEW_WIDTH);
    fill_band(frame, 0, attitude_y, geometry.attitude.x.max(0), geometry.attitude.h as i32, white);
    fill_band(frame, attitude_right, attitude_y, VIEW_WIDTH - attitude_right, geometry.attitude.h as i32, white);
}

fn fill_band(frame: &mut GuiFramebuffer, x: i32, y: i32, width: i32, height: i32, color: Rgb565) {
    if width > 0 && height > 0 {
        common::fill_box(frame, x, y, width as u32, height as u32, color);
    }
}

fn draw_shell(frame: &mut GuiFramebuffer, geometry: Geometry) {
    common::fill_rect(frame, geometry.header, common::dark_blue());
    common::draw_title(frame, "IMU", geometry.header.x + 6, geometry.header.y + 3, common::white());
    common::draw_body(frame, "WAITING", geometry.header.x + 6, geometry.header.y + 20, common::white());
    common::fill_rect(frame, geometry.attitude, common::light_blue());
    draw_border(frame, geometry.attitude);
}

fn draw_header(frame: &mut GuiFramebuffer, area: Rect, imu: &ImuDisplay, attitude: DisplayAttitude) {
    common::fill_rect(frame, area, common::dark_blue());
    common::draw_title(frame, "IMU", area.x + 6, area.y + 3, common::white());
    common::draw_body(frame, status_text(imu.status), area.x + 6, area.y + 19, common::white());

    let mut mag = ArrayString::<24>::new();
    match imu.mag_status {
        sensor::MagStatus::Ready => { let _ = write!(&mut mag, "MAG {}uT", imu.mag_field_ut); }
        sensor::MagStatus::Learning => { let _ = write!(&mut mag, "CAL {}%", imu.mag_calibration); }
        sensor::MagStatus::Disturbed => { let _ = write!(&mut mag, "DIST {}uT", imu.mag_field_ut); }
        sensor::MagStatus::Missing => mag.push_str("MAG MISSING"),
    }
    common::draw_body(frame, mag.as_str(), area.x + 6, area.y + 35, common::light_gray());

    let first_x = area.x + 78;
    let column_width = ((area.w as i32 - 78) / 3).max(1);
    draw_header_value(frame, "ROLL", round_degrees(attitude.roll_deg), first_x, area.y);
    draw_header_value(frame, "PITCH", round_degrees(attitude.pitch_deg), first_x + column_width, area.y);
    draw_header_value(frame, "YAW", round_degrees(attitude.yaw_deg), first_x + column_width * 2, area.y);
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
    common::hline(frame, area.x, area.y + area.h as i32 - 1, width, common::light_gray());
    common::vline(frame, area.x, area.y, height, common::light_gray());
    common::vline(frame, area.x + area.w as i32 - 1, area.y, height, common::light_gray());
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
    gui.absolute_rect(id).unwrap_or_else(|| panic!("{name} layout missing"))
}
