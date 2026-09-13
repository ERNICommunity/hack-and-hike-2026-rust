//! Small example application using the display and IMU capabilities.

#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};

use hack_and_hike::{
    Board,
    capabilities::{
        display::{HEIGHT, Region, WIDTH},
        imu::Status,
    },
};

esp_bootloader_esp_idf::esp_app_desc!();

#[esp_rtos::main]
async fn main(_spawner: Spawner) -> ! {
    let Board {
        mut display,
        mut imu,
        ..
    } = Board::init();

    let full_screen = Region::new(0, 0, WIDTH, HEIGHT);
    let mut screen_color = 0x0000;

    loop {
        if let Some(sample) = imu.latest() {
            screen_color = if sample.status == Status::Running {
                0x07E0 // green
            } else {
                0xF800 // red
            };
        }

        {
            let mut surface = display.surface(full_screen);
            surface.render_scanlines(|_y, pixels| {
                pixels.fill(screen_color);
            });
        }

        Timer::after(Duration::from_millis(50)).await;
    }
}
