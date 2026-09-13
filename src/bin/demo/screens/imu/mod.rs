//! Attitude and heading as a perspective horizon with a compass.

mod compass;
mod horizon;
mod projection;

use core::fmt::Write as _;

use arrayvec::ArrayString;
use embassy_time::{Duration, Instant};
use embedded_graphics::prelude::Point;
use embedded_gui::Rect;
use hack_and_hike::{
    capabilities::{
        display::Surface,
        imu::{Imu, MagStatus, Sample, Status},
    },
    ui::{
        common,
        gui::{self, GuiFramebuffer, GuiSurface},
        theme,
    },
};

use crate::{layout, screens::Screen};

mod generated {
    use embedded_gui::prelude::*;
    embedded_gui::include_gui!("src/bin/demo/screens/imu/imu.kdl");
}

const NODES: usize = 16;
/// Matches the 100 Hz fusion rate; the handle keeps only the newest sample.
const UPDATE_PERIOD: Duration = Duration::from_millis(10);

const HEADER_PADDING: i32 = 6;
const HEADER_TITLE_Y: i32 = 3;
const HEADER_STATUS_Y: i32 = 19;
const HEADER_MAGNETOMETER_Y: i32 = 35;
/// Where the roll/pitch/yaw columns start inside the header.
const HEADER_VALUES_X: i32 = 78;
const VALUE_LABEL_Y: i32 = 3;
const VALUE_Y: i32 = 22;

pub(crate) struct ImuScreen {
    imu: Imu,
    sample: Option<Sample>,
    last_update: Instant,
    gui: &'static mut gui::Context<NODES>,
    header: Rect,
    attitude: Rect,
    dirty: bool,
}

impl ImuScreen {
    pub(crate) fn new(imu: Imu) -> Self {
        let gui = gui::context::<NODES>(layout::CONTENT_WIDTH, layout::CONTENT_HEIGHT);
        let app = generated::ImuApp::build(gui).expect("imu.kdl fits the GUI capacities");
        Self {
            imu,
            sample: None,
            last_update: Instant::now(),
            header: gui::slot(gui, app.widgets.header),
            attitude: gui::slot(gui, app.widgets.attitude),
            gui,
            dirty: true,
        }
    }
}

impl Screen for ImuScreen {
    fn enter(&mut self) {
        self.dirty = true;
    }

    fn update(&mut self, now: Instant) {
        if now - self.last_update < UPDATE_PERIOD {
            return;
        }
        self.last_update = now;
        if let Some(sample) = self.imu.latest() {
            self.sample = Some(sample);
            self.dirty = true;
        }
    }

    fn present(&mut self, gui: &mut GuiSurface, surface: &mut Surface<'_>) {
        if !self.dirty {
            return;
        }
        self.dirty = false;

        let (header, attitude, sample) = (self.header, self.attitude, self.sample);
        gui.present(surface, self.gui, |frame| match sample {
            Some(sample) => {
                let display = projection::display_attitude(&sample.orientation);
                draw_header(frame, header, &sample, display);
                horizon::draw_attitude(frame, attitude, display);
            }
            None => {
                draw_header_frame(frame, header, "WAITING");
                common::fill(frame, attitude, theme::LIGHT_BLUE);
                common::outline(frame, attitude, theme::LIGHT_GRAY);
            }
        });
    }
}

fn draw_header_frame(frame: &mut GuiFramebuffer, area: Rect, status: &str) {
    common::fill(frame, area, theme::DARK_BLUE);
    let x = area.x + HEADER_PADDING;
    common::text(
        frame,
        "IMU",
        Point::new(x, area.y + HEADER_TITLE_Y),
        common::TITLE_FONT,
        theme::WHITE,
    );
    common::text(
        frame,
        status,
        Point::new(x, area.y + HEADER_STATUS_Y),
        common::BODY_FONT,
        theme::WHITE,
    );
}

fn draw_header(
    frame: &mut GuiFramebuffer,
    area: Rect,
    sample: &Sample,
    attitude: projection::DisplayAttitude,
) {
    draw_header_frame(frame, area, status_text(sample.status));

    let mut magnetometer = ArrayString::<24>::new();
    let field = round_to_i32(sample.mag_field_strength_ut);
    let _ = match sample.mag_status {
        MagStatus::Ready => write!(magnetometer, "MAG {field}uT"),
        MagStatus::Learning => write!(magnetometer, "CAL {}%", sample.mag_calibration_percent),
        MagStatus::Disturbed => write!(magnetometer, "DIST {field}uT"),
        MagStatus::Missing => write!(magnetometer, "MAG MISSING"),
    };
    common::text(
        frame,
        &magnetometer,
        Point::new(area.x + HEADER_PADDING, area.y + HEADER_MAGNETOMETER_Y),
        common::BODY_FONT,
        theme::LIGHT_GRAY,
    );

    let column_width = ((area.w as i32 - HEADER_VALUES_X) / 3).max(1);
    let columns = [
        ("ROLL", attitude.roll_deg),
        ("PITCH", attitude.pitch_deg),
        ("YAW", attitude.yaw_deg),
    ];
    for (index, (label, degrees)) in columns.into_iter().enumerate() {
        let x = area.x + HEADER_VALUES_X + column_width * index as i32;
        common::text(
            frame,
            label,
            Point::new(x, area.y + VALUE_LABEL_Y),
            common::BODY_FONT,
            theme::WHITE,
        );
        let mut value = ArrayString::<12>::new();
        let _ = write!(value, "{:+}", round_to_i32(degrees));
        common::text(
            frame,
            &value,
            Point::new(x, area.y + VALUE_Y),
            common::TITLE_FONT,
            theme::WHITE,
        );
    }
}

fn status_text(status: Status) -> &'static str {
    match status {
        Status::Starting => "STARTING",
        Status::Running => "RUNNING",
        Status::Degraded => "DEGRADED",
        Status::Fault => "FAULT",
    }
}

fn round_to_i32(value: f32) -> i32 {
    libm::roundf(value) as i32
}
