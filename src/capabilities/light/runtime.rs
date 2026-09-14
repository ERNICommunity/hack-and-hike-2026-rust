//! CPU1 task polling the LTR-553 over the shared system bus, for both the
//! light and the proximity capability.
//!
//! The sensor measures on its own every 100 ms; the task reads the result at
//! the same rate, so every poll sees a fresh measurement, and publishes the
//! light part to the light handle and the proximity part to the proximity
//! handle.
//!
//! The light sensor's gain is adjusted automatically: raised when the counts
//! are small, lowered before they saturate, so a dark room and daylight both
//! resolve. The counts are converted with the gain the sensor reports along
//! with them, so a change is never applied to the wrong measurement.

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use hack_and_hike_core::light::{self, Channels, DATA_BLOCK_LEN, PROXIMITY_MAX};
use log::warn;

use crate::{
    capabilities::proximity,
    platform::{i2c::SystemI2cBus, registers::AsyncRegisters},
};

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
/// Readings after a gain change that may still show the previous gain. A
/// mismatch beyond that means the sensor lost its configuration (a brown-out
/// resets it to standby at 1x) and is set up again.
const SETTLE_READINGS: u8 = 2;
/// Time between attempts to configure a sensor that does not answer.
const CONFIGURE_RETRY: Duration = Duration::from_secs(1);

/// Start polling the sensor on CPU1, feeding both handles.
pub(crate) fn spawn(
    spawner: &Spawner,
    bus: SystemI2cBus,
    light: Runtime,
    proximity: proximity::Runtime,
) {
    spawner.spawn(poll_task(bus, light, proximity).expect("light task already spawned"));
}

/// Configure the sensor, then read and publish a sample every 100 ms. A
/// failed read is skipped; the first failure is logged, later ones are not,
/// so a sensor that stops answering does not flood the log.
#[embassy_executor::task]
async fn poll_task(bus: SystemI2cBus, light: Runtime, proximity: proximity::Runtime) {
    let mut gain_index = INITIAL_GAIN_INDEX;
    configure_until_it_works(bus, gain_index).await;

    let mut read_failed = false;
    let mut reset_logged = false;
    // Readings still allowed to carry the previous gain after a change.
    let mut settling = SETTLE_READINGS;
    loop {
        Timer::after(POLL_INTERVAL).await;

        let mut block = [0u8; DATA_BLOCK_LEN];
        let read = {
            let mut i2c = bus.lock().await;
            i2c.write_read_async(ltr553::ADDRESS, &[ltr553::DATA_START], &mut block)
                .await
        };
        let reading = match read {
            Ok(()) => {
                read_failed = false;
                light::decode(block)
            }
            Err(error) => {
                if !read_failed {
                    warn!("LTR-553 light sensor read failed: {:?}", error);
                }
                read_failed = true;
                None
            }
        };
        let Some(reading) = reading else {
            continue;
        };

        light.publish(light_sample(reading));
        proximity.publish(proximity_sample(reading));

        // Judge the gain only on a measurement taken at the gain we asked
        // for; the first readings after a change may still be at the old one.
        let Some(reported) = gain_index_of(reading.gain) else {
            continue;
        };
        if reported != gain_index {
            if settling > 0 {
                settling -= 1;
                continue;
            }
            if !reset_logged {
                warn!(
                    "LTR-553 reports gain {}x instead of the requested {}x: configuring it again",
                    reading.gain,
                    ltr553::ALS_GAIN_FACTORS[gain_index]
                );
                reset_logged = true;
            }
            configure_until_it_works(bus, gain_index).await;
            settling = SETTLE_READINGS;
            continue;
        }
        if let Some(next) = better_gain(gain_index, reading.channels) {
            gain_index = next;
            settling = SETTLE_READINGS;
            let set = {
                let mut i2c = bus.lock().await;
                ltr553::set_als_gain(
                    &mut AsyncRegisters::new(&mut *i2c, ltr553::ADDRESS),
                    gain_index,
                )
                .await
            };
            if let Err(error) = set {
                warn!("LTR-553 gain change failed: {:?}", error);
            }
        }
    }
}

/// Configure the sensor, retrying every second until it answers. The probe
/// during bring-up saw the chip, so a failure here is transient.
async fn configure_until_it_works(bus: SystemI2cBus, gain_index: usize) {
    let mut logged = false;
    loop {
        let configured = {
            let mut i2c = bus.lock().await;
            ltr553::configure(
                &mut AsyncRegisters::new(&mut *i2c, ltr553::ADDRESS),
                gain_index,
            )
            .await
        };
        match configured {
            Ok(()) => return,
            Err(error) => {
                if !logged {
                    warn!("LTR-553 light sensor setup failed: {:?}; retrying", error);
                    logged = true;
                }
                Timer::after(CONFIGURE_RETRY).await;
            }
        }
    }
}

/// The index into `ALS_GAIN_CODES` of a gain factor the sensor reports.
fn gain_index_of(factor: u8) -> Option<usize> {
    ltr553::ALS_GAIN_FACTORS.iter().position(|&f| f == factor)
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

/// The light part of one reading: lux from the two channels at the gain
/// they were measured with.
fn light_sample(reading: light::Reading) -> Sample {
    Sample {
        lux: light::lux(reading.channels, reading.gain, ltr553::ALS_INTEGRATION_MS),
    }
}

/// The proximity part of one reading: the raw count, with saturation
/// reported as the maximum, and the count spread evenly over the distance.
fn proximity_sample(reading: light::Reading) -> proximity::Sample {
    let raw = if reading.proximity_saturated {
        PROXIMITY_MAX
    } else {
        reading.proximity
    };
    proximity::Sample {
        percent: light::closeness_percent(
            raw,
            ltr553::PROXIMITY_FAR_COUNT,
            ltr553::PROXIMITY_NEAR_COUNT,
        ),
        raw,
    }
}
