//! CPU1 task that applies brightness requests. It sets the voltage of the
//! backlight rail on the AXP2101 power chip over the shared system I2C bus.

use embassy_executor::Spawner;
use log::warn;

use crate::board::{self, i2c::SystemI2cBus};

use super::Runtime;

/// Start applying brightness requests on CPU1.
pub(crate) fn spawn(spawner: &Spawner, bus: SystemI2cBus, runtime: Runtime) {
    spawner.spawn(apply_task(bus, runtime).expect("backlight task already spawned"));
}

/// Wait for a request, apply it, and repeat. A failed I2C transfer is logged.
/// The failed request is not tried again; the next request is applied as
/// usual.
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
