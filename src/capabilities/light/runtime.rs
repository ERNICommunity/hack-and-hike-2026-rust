//! CPU1 task polling the LTR-553 over the shared system bus.
//!
//! The sensor measures on its own every 100 ms; the task reads the result at
//! the same rate, so every poll sees a fresh measurement.

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use hack_and_hike_core::light::{self, DATA_BLOCK_LEN, PROXIMITY_MAX};
use log::warn;

use crate::platform::{i2c::SystemI2cBus, registers::AsyncRegisters};

use super::{Runtime, Sample, ltr553};

/// Time between two reads, matching the sensor's measurement rate.
const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Start polling the sensor on CPU1.
pub(crate) fn spawn(spawner: &Spawner, bus: SystemI2cBus, runtime: Runtime) {
    spawner.spawn(poll_task(bus, runtime).expect("light task already spawned"));
}

/// Configure the sensor, then read and publish a sample every 100 ms. A
/// failed read is skipped; the first failure is logged, later ones are not,
/// so a sensor that stops answering does not flood the log.
#[embassy_executor::task]
async fn poll_task(bus: SystemI2cBus, runtime: Runtime) {
    let configured = {
        let mut i2c = bus.lock().await;
        ltr553::configure(&mut AsyncRegisters::new(&mut *i2c, ltr553::ADDRESS)).await
    };
    if let Err(error) = configured {
        warn!("LTR-553 light sensor setup failed: {:?}", error);
        return;
    }

    let mut read_failed = false;
    loop {
        Timer::after(POLL_INTERVAL).await;

        let mut block = [0u8; DATA_BLOCK_LEN];
        let read = {
            let mut i2c = bus.lock().await;
            i2c.write_read_async(ltr553::ADDRESS, &[ltr553::DATA_START], &mut block)
                .await
        };
        match read {
            Ok(()) => {
                read_failed = false;
                if let Some(reading) = light::decode(block) {
                    runtime.publish(sample_from(reading));
                }
            }
            Err(error) => {
                if !read_failed {
                    warn!("LTR-553 light sensor read failed: {:?}", error);
                }
                read_failed = true;
            }
        }
    }
}

/// The application's view of one reading: lux from the two channels, and a
/// saturated proximity reported as the maximum count.
fn sample_from(reading: light::Reading) -> Sample {
    Sample {
        lux: light::lux(
            reading.channels,
            ltr553::ALS_GAIN,
            ltr553::ALS_INTEGRATION_MS,
        ),
        proximity: if reading.proximity_saturated {
            PROXIMITY_MAX
        } else {
            reading.proximity
        },
    }
}
