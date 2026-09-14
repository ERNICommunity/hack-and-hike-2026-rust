//! CPU1 task that applies brightness requests over the shared system bus.

use embassy_executor::Spawner;
use log::warn;

use crate::board::{self, i2c::SystemI2cBus};

use super::Runtime;

/// Start applying brightness requests on CPU1.
pub(crate) fn spawn(spawner: &Spawner, bus: SystemI2cBus, runtime: Runtime) {
    spawner.spawn(apply_task(bus, runtime).expect("backlight task already spawned"));
}

/// Wait for a request, apply it, repeat. A failed I2C write is logged and the
/// next request tries again.
#[embassy_executor::task]
async fn apply_task(bus: SystemI2cBus, runtime: Runtime) {
    loop {
        let brightness = runtime.next_request().await;
        let result = {
            let mut i2c = bus.lock().await;
            board::power::set_lcd_backlight(&mut *i2c, brightness.percent()).await
        };
        if let Err(error) = result {
            warn!("LCD brightness update failed: {:?}", error);
        }
    }
}
