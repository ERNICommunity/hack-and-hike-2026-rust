//! CPU1 FT6336 polling.

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use embedded_graphics::prelude::Point;

use crate::platform::{self, i2c::SystemI2cBus};

use super::{Runtime, TouchEvent};

const FT6336_ADDR: u8 = 0x38;
const FT6336_TOUCH_DATA: u8 = 0x02;
const POLL_INTERVAL: Duration = Duration::from_millis(5);

pub(crate) fn spawn(spawner: &Spawner, bus: SystemI2cBus, runtime: Runtime) {
    spawner.spawn(poll_task(bus, runtime).expect("touch task already spawned"));
}

#[derive(Clone, Copy)]
enum Sample {
    Up,
    Down(Point),
}

/// Poll the controller once. `None` when the read failed or the reported
/// position is outside the panel; such samples are simply skipped.
async fn read_sample(bus: SystemI2cBus) -> Option<Sample> {
    let mut data = [0u8; 5];
    {
        let mut i2c = bus.lock().await;
        i2c.write_read_async(FT6336_ADDR, &[FT6336_TOUCH_DATA], &mut data)
            .await
            .ok()?;
    }

    // Low nibble: number of touch points; high nibbles of the coordinate
    // bytes carry event flags.
    if data[0] & 0x0F == 0 {
        return Some(Sample::Up);
    }
    let x = (u16::from(data[1] & 0x0F) << 8) | u16::from(data[2]);
    let y = (u16::from(data[3] & 0x0F) << 8) | u16::from(data[4]);
    if usize::from(x) >= platform::DISPLAY_WIDTH || usize::from(y) >= platform::DISPLAY_HEIGHT {
        return None;
    }

    let (x, y) = platform::logical_display_point(x, y);
    Some(Sample::Down(Point::new(i32::from(x), i32::from(y))))
}

/// Turns raw samples into press, move and release events. Never waits for
/// the application.
#[embassy_executor::task]
async fn poll_task(bus: SystemI2cBus, runtime: Runtime) {
    let mut pressed_at: Option<Point> = None;

    loop {
        match (read_sample(bus).await, pressed_at) {
            (Some(Sample::Down(point)), None) => {
                pressed_at = Some(point);
                runtime.publish(TouchEvent::Pressed(point));
            }
            (Some(Sample::Down(point)), Some(last)) if point != last => {
                pressed_at = Some(point);
                runtime.publish(TouchEvent::Moved(point));
            }
            (Some(Sample::Up), Some(last)) => {
                pressed_at = None;
                runtime.publish(TouchEvent::Released(last));
            }
            _ => {}
        }

        Timer::after(POLL_INTERVAL).await;
    }
}
