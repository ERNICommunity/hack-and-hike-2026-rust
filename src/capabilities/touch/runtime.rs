//! CPU1 task polling the FT6336 touch controller.
//!
//! The FT6336 has an interrupt line. The task does not use it: polling every
//! 5 ms is simpler, needs only one short I2C read, and follows even a quick
//! finger movement.

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use embedded_graphics::prelude::Point;

use hack_and_hike_core::touch::decode_report;

use crate::board::{self, i2c::SystemI2cBus};

use super::{Runtime, TouchEvent};

/// I2C address of the FT6336.
const FT6336_ADDR: u8 = 0x38;
/// First register of the report. The report starts with the number of
/// touches, followed by the position of the first touch point.
const FT6336_REPORT_REGISTER: u8 = 0x02;
/// Wait between two reads of the controller.
const POLL_INTERVAL: Duration = Duration::from_millis(5);

/// Start polling the touch controller on CPU1.
///
/// # Panics
///
/// When the task is already running.
pub(crate) fn spawn(spawner: &Spawner, bus: SystemI2cBus, runtime: Runtime) {
    spawner.spawn(poll_task(bus, runtime).expect("touch task already spawned"));
}

/// What one read of the controller says.
#[derive(Clone, Copy)]
enum Sample {
    /// No finger on the panel.
    Up,
    /// A finger at this display position.
    Down(Point),
}

/// Read the controller once. Returns `None` when the read failed or the
/// reported position is outside the panel. The task skips such samples.
async fn read_sample(bus: SystemI2cBus) -> Option<Sample> {
    let mut report = [0u8; 5];
    {
        let mut i2c = bus.lock().await;
        i2c.write_read_async(FT6336_ADDR, &[FT6336_REPORT_REGISTER], &mut report)
            .await
            .ok()?;
    }

    let Some(raw) = decode_report(report) else {
        return Some(Sample::Up);
    };
    if usize::from(raw.x) >= board::DISPLAY_WIDTH || usize::from(raw.y) >= board::DISPLAY_HEIGHT {
        return None;
    }

    let (x, y) = board::logical_display_point(raw.x, raw.y);
    Some(Sample::Down(Point::new(i32::from(x), i32::from(y))))
}

/// Turn samples into press, move and release events. Never waits for the
/// application.
///
/// A skipped sample (`None`) changes nothing. A finger that stays at the
/// same position gives no event.
#[embassy_executor::task]
async fn poll_task(bus: SystemI2cBus, runtime: Runtime) {
    let mut pressed_at: Option<Point> = None;

    loop {
        match (read_sample(bus).await, pressed_at) {
            // When the queue has no room for the press, `pressed_at` stays
            // `None`. The next sample with a finger tries the press again. So
            // moves and a release never arrive without their press.
            (Some(Sample::Down(point)), None) => {
                if runtime.publish(TouchEvent::Pressed(point)) {
                    pressed_at = Some(point);
                }
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
