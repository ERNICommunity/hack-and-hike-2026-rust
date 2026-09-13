//! A starting point for your own application: copy this file to
//! `src/bin/<your_name>.rs` and build it with `cargo build --release --bin
//! <your_name>`.
//!
//! As it is, the screen is dark blue and turns light blue while you touch it.

#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use hack_and_hike::{
    Board,
    capabilities::{display::SCREEN, touch::TouchEvent},
    ui::theme,
};

esp_bootloader_esp_idf::esp_app_desc!();

#[esp_rtos::main]
async fn main(_spawner: Spawner) -> ! {
    // Keep the handles you need; the rest of the board keeps running anyway.
    let Board {
        mut display,
        mut touch,
        ..
    } = Board::init();

    let mut pressed = false;
    let mut shown = None;

    loop {
        // 1. Read input.
        while let Some(event) = touch.next_event() {
            pressed = !matches!(event, TouchEvent::Released(_));
        }

        // 2. Update your state.
        let color = if pressed {
            theme::LIGHT_BLUE
        } else {
            theme::DARK_BLUE
        };

        // 3. Draw, but only when something changed.
        if shown != Some(color) {
            shown = Some(color);
            display
                .surface(SCREEN)
                .render_scanlines(|_y, row| row.fill(color));
        }

        // Let the rest of the system run. Every loop needs an `.await`.
        Timer::after(Duration::from_millis(10)).await;
    }
}
