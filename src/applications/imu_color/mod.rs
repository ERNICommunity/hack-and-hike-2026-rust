//! Small example application using the display and IMU capabilities.

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};

use crate::{
    capabilities::{
        display::{HEIGHT, Region, WIDTH},
        imu::Status,
    },
    firmware::Bootstrap,
};

pub(crate) async fn run(_spawner: Spawner, bootstrap: Bootstrap) -> ! {
    let Bootstrap {
        mut display,
        mut imu,
        ..
    } = bootstrap;

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
