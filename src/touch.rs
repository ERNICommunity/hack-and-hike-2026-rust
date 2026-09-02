use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex,
    channel::Channel,
    signal::Signal,
};
use embassy_time::{Duration, Timer};

use crate::system_i2c::SystemI2cBus;

const FT6336_ADDR: u8 = 0x38;
const FT6336_TOUCH_DATA: u8 = 0x02;
const SCREEN_WIDTH: u16 = 320;
const SCREEN_HEIGHT: u16 = 240;
const POLL_INTERVAL: Duration = Duration::from_millis(5);

// These outputs cross from CPU1 acquisition to CPU0 presentation, therefore
// they still use CriticalSectionRawMutex. Only the physical I2C bus mutex is
// executor-local.
static TOUCH_EDGES: Channel<CriticalSectionRawMutex, TouchEdge, 8> = Channel::new();
static LATEST_POINT: Signal<CriticalSectionRawMutex, TouchPoint> = Signal::new();

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

pub fn take_latest_point() -> Option<TouchPoint> {
    LATEST_POINT.try_take()
}

pub fn try_take_edge() -> Option<TouchEdge> {
    TOUCH_EDGES.try_receive().ok()
}

async fn read_sample(bus: SystemI2cBus) -> TouchSample {
    let mut data = [0u8; 5];

    // The bus guard remains held for the whole transaction so no future sensor
    // task can interleave bytes on the physical bus. Unlike the previous
    // blocking driver, the I2C transfer itself yields CPU1 while hardware is
    // waiting for bus events.
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
    let point = TouchPoint { x, y };

    if x >= SCREEN_WIDTH || y >= SCREEN_HEIGHT {
        TouchSample::ReadError
    } else {
        TouchSample::Down(point)
    }
}

/// CPU1 touch acquisition. This task never touches Slint and never waits for
/// CPU0 to consume movement samples.
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
                LATEST_POINT.signal(point);
                let _ = TOUCH_EDGES.try_send(TouchEdge::Pressed(point));
            }
            TouchSample::Down(point) if point != last_point => {
                last_point = point;
                LATEST_POINT.signal(point);
            }
            TouchSample::Down(_) => {}
        }

        Timer::after(POLL_INTERVAL).await;
    }
}
