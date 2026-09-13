//! Attitude and heading: a numeric header, an artificial horizon and a
//! compass.

mod horizon;

use core::fmt::Write as _;

use arrayvec::ArrayString;
use embassy_time::{Duration, Instant};
use embedded_graphics::{prelude::Point, primitives::Rectangle};
use hack_and_hike::{
    capabilities::{
        display::Surface,
        imu::{Imu, MagStatus, Sample, Status},
    },
    ui::{Canvas, common, gui, theme},
};

use crate::{layout, screens::Screen};

mod generated {
    use embedded_gui::prelude::*;
    embedded_gui::include_gui!("src/bin/demo/screens/imu/imu.kdl");
}

const NODES: usize = 16;
const _: () = assert!(generated::ImuApp::WIDTH == layout::CONTENT_SIZE.width);
const _: () = assert!(generated::ImuApp::HEIGHT == layout::CONTENT_SIZE.height);
/// Matches the 100 Hz fusion rate; the handle keeps only the newest sample.
const UPDATE_PERIOD: Duration = Duration::from_millis(10);

const HEADER_PADDING: i32 = 6;
const HEADER_TITLE_Y: i32 = 3;
const HEADER_STATUS_Y: i32 = 19;
const HEADER_MAGNETOMETER_Y: i32 = 35;
/// Where the roll/pitch/heading columns start inside the header.
const HEADER_VALUES_X: i32 = 78;
const VALUE_LABEL_Y: i32 = 3;
const VALUE_Y: i32 = 22;

pub(crate) struct ImuScreen {
    imu: Imu,
    sample: Option<Sample>,
    last_update: Instant,
    gui: &'static mut gui::Context<NODES>,
    header: Rectangle,
    attitude: Rectangle,
    dirty: bool,
}

impl ImuScreen {
    pub(crate) fn new(imu: Imu) -> Self {
        let gui = gui::context::<NODES>(layout::CONTENT_SIZE.width, layout::CONTENT_SIZE.height);
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

    fn present(&mut self, canvas: &mut Canvas, surface: &mut Surface<'_>) {
        if !self.dirty {
            return;
        }
        self.dirty = false;

        canvas.clear(theme::WHITE);
        gui::render(self.gui, canvas);
        match &self.sample {
            Some(sample) => {
                draw_header(canvas, self.header, sample);
                horizon::draw(canvas, self.attitude, sample);
            }
            None => {
                draw_header_frame(canvas, self.header, "WAITING");
                canvas.fill(self.attitude, theme::LIGHT_BLUE);
            }
        }
        canvas.show(surface);
    }
}

fn draw_header_frame(canvas: &mut Canvas, area: Rectangle, status: &str) {
    canvas.fill(area, theme::DARK_BLUE);
    let x = area.top_left.x + HEADER_PADDING;
    common::text(
        canvas,
        "IMU",
        Point::new(x, area.top_left.y + HEADER_TITLE_Y),
        common::TITLE_FONT,
        theme::WHITE,
    );
    common::text(
        canvas,
        status,
        Point::new(x, area.top_left.y + HEADER_STATUS_Y),
        common::BODY_FONT,
        theme::WHITE,
    );
}

fn draw_header(canvas: &mut Canvas, area: Rectangle, sample: &Sample) {
    draw_header_frame(canvas, area, status_text(sample.status));

    let mut magnetometer = ArrayString::<24>::new();
    let field = round(sample.mag_field_strength_ut);
    let _ = match sample.mag_status {
        MagStatus::Ready => write!(magnetometer, "MAG {field}uT"),
        MagStatus::Learning => write!(magnetometer, "CAL {}%", sample.mag_calibration_percent),
        MagStatus::Disturbed => write!(magnetometer, "DIST {field}uT"),
        MagStatus::Missing => write!(magnetometer, "MAG MISSING"),
    };
    common::text(
        canvas,
        &magnetometer,
        area.top_left + Point::new(HEADER_PADDING, HEADER_MAGNETOMETER_Y),
        common::BODY_FONT,
        theme::LIGHT_GRAY,
    );

    let attitude = sample.attitude;
    let column_width = ((area.size.width as i32 - HEADER_VALUES_X) / 3).max(1);
    let columns = [
        ("ROLL", attitude.roll_deg),
        ("PITCH", attitude.pitch_deg),
        ("HDG", attitude.heading_deg),
    ];
    for (index, (label, degrees)) in columns.into_iter().enumerate() {
        let x = area.top_left.x + HEADER_VALUES_X + column_width * index as i32;
        common::text(
            canvas,
            label,
            Point::new(x, area.top_left.y + VALUE_LABEL_Y),
            common::BODY_FONT,
            theme::WHITE,
        );
        let mut value = ArrayString::<12>::new();
        let _ = write!(value, "{:+}", round(degrees));
        common::text(
            canvas,
            &value,
            Point::new(x, area.top_left.y + VALUE_Y),
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

fn round(value: f32) -> i32 {
    libm::roundf(value) as i32
}
