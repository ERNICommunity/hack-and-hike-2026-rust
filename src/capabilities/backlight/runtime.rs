//! CPU1 task that applies brightness requests over the shared system bus.

use embassy_executor::Spawner;
use log::warn;

use crate::platform::{self, i2c::SystemI2cBus};

use super::Runtime;

pub(crate) fn spawn(spawner: &Spawner, bus: SystemI2cBus, runtime: Runtime) {
    spawner.spawn(apply_task(bus, runtime).expect("backlight task already spawned"));
}

#[embassy_executor::task]
async fn apply_task(bus: SystemI2cBus, runtime: Runtime) {
    loop {
        let brightness = runtime.next_request().await;
        let result = {
            let mut i2c = bus.lock().await;
            platform::power::set_lcd_backlight(&mut *i2c, brightness.percent()).await
        };
        if let Err(error) = result {
            warn!("LCD brightness update failed: {:?}", error);
        }
    }
}
