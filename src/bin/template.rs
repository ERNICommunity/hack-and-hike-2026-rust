//! A starting point for your own application: copy this file to
//! `src/bin/<your_name>.rs` and build it with `cargo build --release --bin
//! <your_name>`.
//!
//! As it is, a light blue spot follows your finger on a dark blue screen.

#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use embedded_graphics::{
    prelude::*,
    primitives::{Circle, PrimitiveStyle},
};
use hack_and_hike::{
    Board,
    capabilities::{
        display::{SCREEN, SIZE},
        touch::TouchEvent,
    },
    ui::{Canvas, theme},
};

// Writes the application descriptor the bootloader checks before starting
// the firmware. Every application needs this line exactly once.
esp_bootloader_esp_idf::esp_app_desc!();

/// Size of the spot under the finger, in pixels.
const SPOT_DIAMETER: u32 = 120;

/// The entry point. `async` because the loop waits with `.await`; `-> !`
/// because firmware never returns.
#[esp_rtos::main]
async fn main(_spawner: Spawner) -> ! {
    // Keep the handles you need; the rest of the board keeps running anyway.
    let Board {
        mut display,
        mut touch,
        ..
    } = Board::init();

    // Draw into the canvas, then show it: only what changed is sent.
    let mut canvas = Canvas::new(SIZE);
    canvas.clear(theme::DARK_BLUE);
    canvas.show(&mut display.surface(SCREEN));

    let mut finger: Option<Point> = None;
    let mut shown = None;

    loop {
        // 1. Read input.
        while let Some(event) = touch.next_event() {
            finger = match event {
                TouchEvent::Pressed(point) | TouchEvent::Moved(point) => Some(point),
                TouchEvent::Released(_) => None,
            };
        }

        // 2. Update your state and draw, but only when something changed.
        if finger != shown {
            shown = finger;
            canvas.clear(theme::DARK_BLUE);
            if let Some(point) = finger {
                let Ok(()) = Circle::with_center(point, SPOT_DIAMETER)
                    .into_styled(PrimitiveStyle::with_fill(theme::LIGHT_BLUE))
                    .draw(&mut canvas);
            }
            canvas.show(&mut display.surface(SCREEN));
        }

        // 3. Let the rest of the system run. Every loop needs an `.await`.
        Timer::after(Duration::from_millis(10)).await;
    }
}
