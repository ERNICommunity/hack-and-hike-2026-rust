//! CPU1 task that switches the camera's automatic exposure and white
//! balance on and off, over the shared system I2C bus. It starts only when
//! the board has a camera, and it does nothing until an application calls
//! [`Camera::set_auto_adjust`](super::Camera::set_auto_adjust).

use embassy_executor::Spawner;
use log::warn;

use crate::board::i2c::SystemI2cBus;

use super::{Runtime, gc0308};

/// Start applying auto adjust requests on CPU1.
pub(crate) fn spawn(spawner: &Spawner, bus: SystemI2cBus, runtime: Runtime) {
    spawner.spawn(control_task(bus, runtime).expect("camera control task already spawned"));
}

/// Wait for a request, apply it, and repeat. A failed I2C transfer is logged.
/// The failed request is not tried again; the next request is applied as
/// usual.
#[embassy_executor::task]
async fn control_task(bus: SystemI2cBus, runtime: Runtime) {
    loop {
        let enabled = runtime.next_request().await;
        let result = {
            let mut i2c = bus.lock().await;
            gc0308::set_auto_adjust(&mut *i2c, enabled).await
        };
        if let Err(error) = result {
            warn!("Camera auto adjust update failed: {:?}", error);
        }
    }
}
