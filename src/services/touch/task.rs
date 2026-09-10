//! CPU1 FT6336 polling and touch-event publication.

use embassy_time::{Duration, Timer};

use crate::{
    platform::{board, i2c::SystemI2cBus},
    support::diagnostics,
};

use super::{TouchEdge, TouchPoint, channels::Runtime};

const FT6336_ADDR: u8 = 0x38;
const FT6336_TOUCH_DATA: u8 = 0x02;
const POLL_INTERVAL: Duration = Duration::from_millis(5);

#[derive(Clone, Copy)]
enum TouchSample {
    Up,
    Down(TouchPoint),
    ReadError,
}

async fn read_sample(bus: SystemI2cBus) -> TouchSample {
    let mut data = [0u8; 5];

    let result = {
        let mut i2c = bus.lock().await;
        i2c.write_read_async(FT6336_ADDR, &[FT6336_TOUCH_DATA], &mut data)
            .await
    };

    if result.is_err() {
        return TouchSample::ReadError;
    }

    if data[0] & 0x0F == 0 {
        return TouchSample::Up;
    }

    let x = (u16::from(data[1] & 0x0F) << 8) | u16::from(data[2]);
    let y = (u16::from(data[3] & 0x0F) << 8) | u16::from(data[4]);

    if usize::from(x) >= board::DISPLAY_WIDTH || usize::from(y) >= board::DISPLAY_HEIGHT {
        return TouchSample::ReadError;
    }

    let (x, y) = board::logical_display_point(x, y);
    TouchSample::Down(TouchPoint { x, y })
}

/// CPU1 touch acquisition. This task never owns presentation state and never
/// waits for CPU0 to consume movement samples.
#[embassy_executor::task]
pub(crate) async fn capture_task(bus: SystemI2cBus, runtime: Runtime) {
    let mut pressed = false;
    let mut last_point = TouchPoint { x: 0, y: 0 };

    loop {
        match read_sample(bus).await {
            TouchSample::ReadError => diagnostics::record_touch_read_error(),
            TouchSample::Up if pressed => {
                pressed = false;
                if !runtime.try_publish_edge(TouchEdge::Released(last_point)) {
                    diagnostics::record_touch_edge_drop();
                }
            }
            TouchSample::Up => {}
            TouchSample::Down(point) if !pressed => {
                pressed = true;
                last_point = point;
                runtime.publish_point(point);
                if !runtime.try_publish_edge(TouchEdge::Pressed(point)) {
                    diagnostics::record_touch_edge_drop();
                }
            }
            TouchSample::Down(point) if point != last_point => {
                last_point = point;
                runtime.publish_point(point);
            }
            TouchSample::Down(_) => {}
        }

        Timer::after(POLL_INTERVAL).await;
    }
}
