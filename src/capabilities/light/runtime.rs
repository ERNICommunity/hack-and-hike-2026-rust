//! CPU1 task polling the LTR-553 over the shared system bus.
//!
//! The sensor measures on its own every 100 ms; the task reads the result at
//! the same rate, so every poll sees a fresh measurement.
//!
//! The light sensor's gain is adjusted automatically: raised when the counts
//! are small, lowered before they saturate, so a dark room and daylight both
//! resolve. The counts are converted with the gain the sensor reports along
//! with them, so a change is never applied to the wrong measurement.

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use hack_and_hike_core::light::{self, Channels, DATA_BLOCK_LEN, PROXIMITY_MAX};
use log::warn;

use crate::platform::{i2c::SystemI2cBus, registers::AsyncRegisters};

use super::{Runtime, Sample, ltr553};

/// Time between two reads, matching the sensor's measurement rate.
const POLL_INTERVAL: Duration = Duration::from_millis(100);
/// Raise the gain when the larger channel count is below this.
const GAIN_UP_BELOW: u16 = 500;
/// Lower the gain when the larger channel count is above this: close to the
/// 65535 the counts saturate at.
const GAIN_DOWN_ABOVE: u16 = 60000;
/// The gain to start with: 8x, in the middle of the range.
const INITIAL_GAIN_INDEX: usize = 3;

/// Start polling the sensor on CPU1.
pub(crate) fn spawn(spawner: &Spawner, bus: SystemI2cBus, runtime: Runtime) {
    spawner.spawn(poll_task(bus, runtime).expect("light task already spawned"));
}

/// Configure the sensor, then read and publish a sample every 100 ms. A
/// failed read is skipped; the first failure is logged, later ones are not,
/// so a sensor that stops answering does not flood the log.
#[embassy_executor::task]
async fn poll_task(bus: SystemI2cBus, runtime: Runtime) {
    let mut gain_index = INITIAL_GAIN_INDEX;
    let configured = {
        let mut i2c = bus.lock().await;
        ltr553::configure(
            &mut AsyncRegisters::new(&mut *i2c, ltr553::ADDRESS),
            gain_index,
        )
        .await
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
                    if let Some(next) = better_gain(gain_index, reading.channels) {
                        gain_index = next;
                        let mut i2c = bus.lock().await;
                        let set = ltr553::set_als_gain(
                            &mut AsyncRegisters::new(&mut *i2c, ltr553::ADDRESS),
                            gain_index,
                        )
                        .await;
                        if let Err(error) = set {
                            warn!("LTR-553 gain change failed: {:?}", error);
                        }
                    }
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

/// The next gain index to use, if the counts call for a change: one step up
/// when both channels are small, one step down when either is close to
/// saturating. The thresholds are far enough apart that a change never
/// triggers the opposite one.
fn better_gain(gain_index: usize, channels: Channels) -> Option<usize> {
    let largest = channels.ch0.max(channels.ch1);
    if largest > GAIN_DOWN_ABOVE && gain_index > 0 {
        Some(gain_index - 1)
    } else if largest < GAIN_UP_BELOW && gain_index + 1 < ltr553::ALS_GAIN_CODES.len() {
        Some(gain_index + 1)
    } else {
        None
    }
}

/// The application's view of one reading: lux from the two channels at the
/// gain they were measured with, the raw proximity count with saturation
/// reported as the maximum, and the count spread evenly over the distance.
fn sample_from(reading: light::Reading) -> Sample {
    let raw_proximity = if reading.proximity_saturated {
        PROXIMITY_MAX
    } else {
        reading.proximity
    };
    Sample {
        lux: light::lux(reading.channels, reading.gain, ltr553::ALS_INTEGRATION_MS),
        proximity: light::closeness_percent(
            raw_proximity,
            ltr553::PROXIMITY_FAR_COUNT,
            ltr553::PROXIMITY_NEAR_COUNT,
        ),
        raw_proximity,
    }
}
