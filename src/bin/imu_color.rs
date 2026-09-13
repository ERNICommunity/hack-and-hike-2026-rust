//! The smallest example: the whole screen is a compass-calibration gauge.
//! It is red while the compass is uncalibrated and turns orange, then yellow,
//! then green as the calibration completes, with the sensor's own numbers on
//! a panel in the middle.

#![no_std]
#![no_main]

use core::fmt::Write as _;

use arrayvec::ArrayString;
use embassy_executor::Spawner;
use embassy_time::{Duration, Instant, Timer};
use embedded_graphics::{
    pixelcolor::Rgb565,
    prelude::{Point, RgbColor as _},
};
use embedded_gui::Rect;

use hack_and_hike::{
    Board,
    capabilities::{
        display::{HEIGHT, Region, WIDTH},
        imu::{MagStatus, Sample, Status},
    },
    ui::{
        common::{self, Lines},
        gui::{GuiFramebuffer, GuiSurface},
        theme,
    },
};

esp_bootloader_esp_idf::esp_app_desc!();

/// The sensor publishes 100 samples per second. Redrawing that often would
/// keep the display busy for nothing, so the screen refreshes four times a
/// second at most.
const REDRAW_PERIOD: Duration = Duration::from_millis(250);

/// The background colour at each calibration percentage, blended in between.
/// Channels are RGB565: red and blue run 0..=31, green 0..=63.
const CALIBRATION_COLORS: [(u8, (u8, u8, u8)); 4] = [
    (0, (31, 0, 0)),   // red
    (50, (31, 41, 0)), // orange
    (75, (31, 63, 0)), // yellow
    (100, (0, 63, 0)), // green
];
/// Above this brightness, dark text reads better than white.
const LIGHT_BACKGROUND_PERCENT: u32 = 55;

const TITLE_AT: Point = Point::new(12, 10);
const PANEL_X: i32 = 12;
const PANEL_Y: i32 = 34;
const PANEL_WIDTH: u32 = WIDTH as u32 - 2 * PANEL_X as u32;
const PANEL_HEIGHT: u32 = HEIGHT as u32 - PANEL_Y as u32 - PANEL_X as u32;
const TEXT_AT: Point = Point::new(PANEL_X + 12, PANEL_Y + 12);

#[esp_rtos::main]
async fn main(_spawner: Spawner) -> ! {
    let Board {
        mut display,
        mut imu,
        ..
    } = Board::init();

    let full_screen = Region::new(0, 0, WIDTH, HEIGHT);
    let mut screen = GuiSurface::new(WIDTH, HEIGHT);
    let mut shown = None;
    let mut last_redraw = Instant::now();

    screen.present_custom(&mut display.surface(full_screen), |frame| draw(frame, None));

    loop {
        if let Some(sample) = imu.latest() {
            let reading = Reading::from_sample(&sample);
            if shown != Some(reading) && Instant::now() - last_redraw >= REDRAW_PERIOD {
                shown = Some(reading);
                last_redraw = Instant::now();
                screen.present_custom(&mut display.surface(full_screen), |frame| {
                    draw(frame, Some(reading));
                });
            }
        }

        Timer::after(Duration::from_millis(10)).await;
    }
}

/// What the screen shows, rounded so that sensor noise does not cause a
/// redraw on every sample.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Reading {
    status: Status,
    magnetometer: MagStatus,
    calibration_percent: u8,
    roll_deg: i32,
    pitch_deg: i32,
    heading_deg: i32,
}

impl Reading {
    fn from_sample(sample: &Sample) -> Self {
        let orientation = sample.orientation;
        Self {
            status: sample.status,
            magnetometer: sample.mag_status,
            calibration_percent: sample.mag_calibration_percent,
            roll_deg: round(orientation.roll_deg),
            pitch_deg: round(orientation.pitch_deg),
            heading_deg: round(orientation.yaw_deg),
        }
    }
}

fn draw(frame: &mut GuiFramebuffer, reading: Option<Reading>) {
    // Without a sample nothing is calibrated yet, so the gauge is empty.
    let calibration = reading.map_or(0, |reading| reading.calibration_percent);
    let background = calibration_color(calibration);
    let screen = Rect::new(0, 0, WIDTH as u32, HEIGHT as u32);
    common::fill(frame, screen, background);
    common::text(
        frame,
        "IMU COLOUR",
        TITLE_AT,
        common::TITLE_FONT,
        readable_on(background),
    );
    common::fill(
        frame,
        Rect::new(PANEL_X, PANEL_Y, PANEL_WIDTH, PANEL_HEIGHT),
        theme::WHITE,
    );

    let mut lines = Lines::new(frame, TEXT_AT);
    lines.line("The screen follows the compass", theme::CHARCOAL);
    lines.line("calibration: red at 0 %, orange at", theme::CHARCOAL);
    lines.line("50 %, yellow at 75 %, green at 100 %.", theme::CHARCOAL);
    lines.line("", theme::CHARCOAL);

    let Some(reading) = reading else {
        lines.line("Waiting for the first sample...", theme::DARK_GRAY);
        return;
    };

    let mut text = ArrayString::<48>::new();
    lines.line(
        match reading.status {
            Status::Starting => "Sensor        starting up",
            Status::Running => "Sensor        running",
            Status::Degraded => "Sensor        read errors",
            Status::Fault => "Sensor        restarting",
        },
        theme::CHARCOAL,
    );
    lines.line(
        match reading.magnetometer {
            MagStatus::Missing => "Compass       not answering",
            MagStatus::Learning => "Compass       calibrating",
            MagStatus::Ready => "Compass       ready",
            MagStatus::Disturbed => "Compass       disturbed",
        },
        theme::CHARCOAL,
    );

    let _ = write!(text, "Calibration   {} %", reading.calibration_percent);
    lines.line(&text, theme::CHARCOAL);
    text.clear();
    let _ = write!(
        text,
        "Roll / pitch  {:+} / {:+}",
        reading.roll_deg, reading.pitch_deg
    );
    lines.line(&text, theme::CHARCOAL);
    text.clear();
    let _ = write!(text, "Heading       {:+}", reading.heading_deg);
    lines.line(&text, theme::CHARCOAL);

    lines.line("", theme::CHARCOAL);
    if reading.calibration_percent < 100 {
        lines.line("Turn the board slowly in every", theme::DARK_GRAY);
        lines.line("direction to calibrate the", theme::DARK_GRAY);
        lines.line("compass to 100 %.", theme::DARK_GRAY);
    } else {
        lines.line("The compass is calibrated, so the", theme::DARK_GRAY);
        lines.line("heading points at magnetic north.", theme::DARK_GRAY);
    }
}

fn round(degrees: f32) -> i32 {
    libm::roundf(degrees) as i32
}

/// The gauge colour for a calibration percentage, blended between the stops
/// in [`CALIBRATION_COLORS`].
fn calibration_color(percent: u8) -> Rgb565 {
    let percent = percent.min(100);
    let (low, high) = CALIBRATION_COLORS
        .windows(2)
        .map(|stops| (stops[0], stops[1]))
        .find(|(low, high)| (low.0..=high.0).contains(&percent))
        .unwrap_or((CALIBRATION_COLORS[0], CALIBRATION_COLORS[0]));

    let span = u32::from(high.0 - low.0);
    let position = u32::from(percent - low.0);
    Rgb565::new(
        blend(low.1.0, high.1.0, position, span),
        blend(low.1.1, high.1.1, position, span),
        blend(low.1.2, high.1.2, position, span),
    )
}

/// `from` at `position` 0, `to` at `position` `span`.
fn blend(from: u8, to: u8, position: u32, span: u32) -> u8 {
    if span == 0 {
        return to;
    }
    let (from, to) = (u32::from(from), u32::from(to));
    let value = if to >= from {
        from + ((to - from) * position + span / 2) / span
    } else {
        from - ((from - to) * position + span / 2) / span
    };
    value as u8
}

/// White or charcoal, whichever reads better on `background`.
fn readable_on(background: Rgb565) -> Rgb565 {
    // Brightness in percent, weighting the channels the way an eye does.
    let brightness = (30 * u32::from(background.r())) / 31
        + (59 * u32::from(background.g())) / 63
        + (11 * u32::from(background.b())) / 31;
    if brightness >= LIGHT_BACKGROUND_PERCENT {
        theme::CHARCOAL
    } else {
        theme::WHITE
    }
}
