//! What a panic looks like, and how to find the code that caused it.
//!
//! The screen shows four bands. Tapping one of the three coloured bands logs
//! its name. The red band on the right has no name, and looking it up is a
//! bug on purpose: the lookup indexes past the end of an array, and the
//! program panics.
//!
//! The panic handler prints the message and a backtrace to the serial log,
//! then stops the program; the screen keeps its last picture. The backtrace
//! is a list of bare addresses. Copy it from the serial log and turn it into
//! function names and lines with:
//!
//! ```bash
//! ./scripts/backtrace.sh panic_backtrace
//! ```

#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use embedded_graphics::{
    pixelcolor::Rgb565,
    prelude::*,
    primitives::{PrimitiveStyle, Rectangle},
};
use hack_and_hike::{
    Board,
    capabilities::{
        display::{SCREEN, SIZE},
        touch::TouchEvent,
    },
    ui::{Canvas, common, theme},
};

esp_bootloader_esp_idf::esp_app_desc!();

/// How many bands the screen is split into, left to right.
const BANDS: u32 = 4;
/// The colour of each band.
const COLOURS: [Rgb565; BANDS as usize] = [
    theme::DARK_BLUE,
    theme::LIGHT_BLUE,
    theme::DARK_GRAY,
    theme::rgb(0xC0392B),
];
/// The name of each band. The bug: there are only three names for four
/// bands.
const NAMES: [&str; 3] = ["DARK BLUE", "LIGHT BLUE", "GRAY"];

#[esp_rtos::main]
async fn main(_spawner: Spawner) -> ! {
    let Board {
        mut display,
        mut touch,
        ..
    } = Board::init();

    let mut canvas = Canvas::new(SIZE);
    draw(&mut canvas);
    canvas.show(&mut display.surface(SCREEN));
    log::info!("Tap a band. The red one panics.");

    loop {
        while let Some(event) = touch.next_event() {
            if let TouchEvent::Pressed(point) = event {
                on_tap(point);
            }
        }

        Timer::after(Duration::from_millis(10)).await;
    }
}

/// Log the name of the band under `point`.
fn on_tap(point: Point) {
    let band = band_at(point);
    log::info!("Tapped band {band}: {}", band_name(band));
}

/// Which band `point` lies in, from 0 on the left.
fn band_at(point: Point) -> usize {
    let band = point.x.max(0) as u32 * BANDS / SIZE.width;
    band.min(BANDS - 1) as usize
}

/// The name of a band. Panics for the red band: `NAMES` has no entry for it.
fn band_name(band: usize) -> &'static str {
    NAMES[band]
}

/// Four bands side by side, the coloured ones labelled with their names.
fn draw(canvas: &mut Canvas) {
    let width = SIZE.width / BANDS;
    for (band, colour) in COLOURS.into_iter().enumerate() {
        let area = Rectangle::new(
            Point::new(band as i32 * width as i32, 0),
            Size::new(width, SIZE.height),
        );
        let Ok(()) = area
            .into_styled(PrimitiveStyle::with_fill(colour))
            .draw(canvas);
        // Not `band_name`: that would panic while drawing the red band.
        let label = NAMES.get(band).copied().unwrap_or("PANIC");
        common::centered_text(canvas, area, label, common::BODY_FONT, theme::WHITE);
    }
}
