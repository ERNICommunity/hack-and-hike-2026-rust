use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex,
    channel::Channel,
    signal::Signal,
};
use embassy_time::{Duration, Timer};

use crate::system_i2c::SystemI2cBus;

const FT6336_ADDR: u8 = 0x38;
const SCREEN_WIDTH: u16 = 320;
const SCREEN_HEIGHT: u16 = 240;
const POLL_INTERVAL_MS: u64 = 5;

// Only edge events are queued. Motion is a latest-value signal below, so a
// blocked UI never creates an unbounded/stale queue of pointer movements.
static TOUCH_EDGES: Channel<CriticalSectionRawMutex, TouchEdge, 8> = Channel::new();

// Movement is overwrite-with-latest state. CPU1 never waits for CPU0 to
// consume an older position.
static LATEST_POINT: Signal<CriticalSectionRawMutex, TouchPoint> = Signal::new();

#[derive(Clone, Copy, Debug)]
pub struct TouchPoint {
    pub x: u16,
    pub y: u16,
}

#[derive(Clone, Copy, Debug)]
pub enum TouchEdge {
    Pressed(TouchPoint),
    Released(TouchPoint),
}

#[derive(Clone, Copy)]
enum TouchSample {
    Up,
    Down(TouchPoint),
    ReadError,
}

fn publish_latest_point(point: TouchPoint) {
    LATEST_POINT.signal(point);
}

/// Returns the newest movement sample since the previous call, if any.
///
/// There is intentionally no queue for move events: CPU0 only needs the most
/// recent finger position after it becomes available again.
pub fn take_latest_point() -> Option<TouchPoint> {
    LATEST_POINT.try_take()
}

/// Nonblocking CPU0-side edge receive.
pub fn try_take_edge() -> Option<TouchEdge> {
    TOUCH_EDGES.try_receive().ok()
}

async fn read_sample(bus: SystemI2cBus) -> TouchSample {
    let mut data = [0u8; 5];

    let result = {
        let mut i2c = bus.lock().await;
        i2c.write_read(FT6336_ADDR, &[0x02], &mut data)
    };

    if result.is_err() {
        return TouchSample::ReadError;
    }

    let touch_count = data[0] & 0x0F;
    if touch_count == 0 {
        return TouchSample::Up;
    }

    let x = (((data[1] & 0x0F) as u16) << 8) | data[2] as u16;
    let y = (((data[3] & 0x0F) as u16) << 8) | data[4] as u16;

    if x >= SCREEN_WIDTH || y >= SCREEN_HEIGHT {
        return TouchSample::ReadError;
    }

    TouchSample::Down(TouchPoint { x, y })
}

/// CPU1 touch acquisition task.
///
/// This task never sees a Slint type and never waits for the UI. Press/release
/// edges are sent with try_send(), while move samples overwrite LATEST_POINT.
#[embassy_executor::task]
pub async fn capture_task(bus: SystemI2cBus) {
    let mut pressed = false;
    let mut last_point = TouchPoint { x: 0, y: 0 };

    loop {
        match read_sample(bus).await {
            TouchSample::ReadError => {}
            TouchSample::Up if pressed => {
                pressed = false;
                let _ = TOUCH_EDGES.try_send(TouchEdge::Released(last_point));
            }
            TouchSample::Up => {}
            TouchSample::Down(point) if !pressed => {
                pressed = true;
                last_point = point;
                publish_latest_point(point);
                let _ = TOUCH_EDGES.try_send(TouchEdge::Pressed(point));
            }
            TouchSample::Down(point) => {
                if point.x != last_point.x || point.y != last_point.y {
                    last_point = point;
                    publish_latest_point(point);
                }
            }
        }

        Timer::after(Duration::from_millis(POLL_INTERVAL_MS)).await;
    }
}
