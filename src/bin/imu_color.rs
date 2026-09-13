//! The smallest application: the screen shows how far the compass calibration
//! has come. Red at the start, orange from 50 %, yellow from 75 % and green
//! once the compass is calibrated. Turn the board slowly in every direction.

#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use embedded_graphics::{pixelcolor::Rgb565, prelude::RgbColor as _};
use hack_and_hike::{Board, capabilities::display::SCREEN};

esp_bootloader_esp_idf::esp_app_desc!();

#[esp_rtos::main]
async fn main(_spawner: Spawner) -> ! {
    let Board {
        mut display,
        mut imu,
        ..
    } = Board::init();
    let mut shown = None;

    loop {
        if let Some(sample) = imu.latest() {
            let color = calibration_color(sample.mag_calibration_percent);
            if shown != Some(color) {
                shown = Some(color);
                display
                    .surface(SCREEN)
                    .render_scanlines(|_y, row| row.fill(color));
            }
        }

        Timer::after(Duration::from_millis(10)).await;
    }
}

/// One fixed colour per stage of the calibration.
fn calibration_color(percent: u8) -> Rgb565 {
    match percent {
        0..50 => Rgb565::RED,
        50..75 => Rgb565::new(31, 32, 0), // orange
        75..100 => Rgb565::YELLOW,
        _ => Rgb565::GREEN,
    }
}
