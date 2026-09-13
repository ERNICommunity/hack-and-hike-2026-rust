//! CPU1 FT6336 polling and touch-event publication.

use embassy_time::{Duration, Timer};

use crate::platform::{self, i2c::SystemI2cBus};

use super::{TouchEdge, TouchPoint, channels::Runtime};

const FT6336_ADDR: u8 = 0x38;
const FT6336_TOUCH_DATA: u8 = 0x02;
const POLL_INTERVAL: Duration = Duration::from_millis(5);

#[derive(Clone, Copy)]
enum TouchSample {
    Up,
    Down(TouchPoint),
}

/// Poll the controller once. `None` when the read failed or the reported
/// position is outside the panel; such samples are simply skipped.
async fn read_sample(bus: SystemI2cBus) -> Option<TouchSample> {
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
        return Some(TouchSample::Up);
    }
    let x = (u16::from(data[1] & 0x0F) << 8) | u16::from(data[2]);
    let y = (u16::from(data[3] & 0x0F) << 8) | u16::from(data[4]);
    if usize::from(x) >= platform::DISPLAY_WIDTH || usize::from(y) >= platform::DISPLAY_HEIGHT {
        return None;
    }

    let (x, y) = platform::logical_display_point(x, y);
    Some(TouchSample::Down(TouchPoint { x, y }))
}

/// CPU1 touch acquisition. Publishes press/release edges and finger movement;
/// never waits for the application.
#[embassy_executor::task]
pub(crate) async fn capture_task(bus: SystemI2cBus, runtime: Runtime) {
    let mut pressed = false;
    let mut last_point = TouchPoint { x: 0, y: 0 };

    loop {
        match read_sample(bus).await {
            Some(TouchSample::Up) if pressed => {
                pressed = false;
                runtime.publish_edge(TouchEdge::Released(last_point));
            }
            Some(TouchSample::Down(point)) if !pressed => {
                pressed = true;
                last_point = point;
                runtime.publish_point(point);
                runtime.publish_edge(TouchEdge::Pressed(point));
            }
            Some(TouchSample::Down(point)) if point != last_point => {
                last_point = point;
                runtime.publish_point(point);
            }
            Some(TouchSample::Up | TouchSample::Down(_)) | None => {}
        }

        Timer::after(POLL_INTERVAL).await;
    }
}
