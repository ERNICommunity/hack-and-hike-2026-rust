//! CPU1 task that polls the LTR-553 over the shared system I2C bus, for both
//! the light and the proximity capability.
//!
//! The LTR-553 contains an ambient light sensor (ALS) and a proximity sensor
//! (PS). It measures by itself every 100 ms. The task reads the result at
//! about the same rate, so most reads get a new measurement. The task
//! publishes the light part to the light handle and the proximity part to
//! the proximity handle.
//!
//! The task changes the gain of the light sensor by itself. It raises the
//! gain when the counts are small, and lowers it before the counts saturate
//! (reach their maximum). So the sensor gives useful values both in a dark
//! room and in daylight. The sensor reports the gain of each measurement
//! together with the counts. The task converts the counts with this reported
//! gain, so a gain change never uses the wrong factor for a measurement.

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use hack_and_hike_core::light::{self, Channels, DATA_BLOCK_LEN, PROXIMITY_MAX};
use log::warn;

use crate::{
    board::{i2c::SystemI2cBus, registers::AsyncRegisters},
    capabilities::proximity,
};

use super::{Runtime, Sample, ltr553};

/// Wait between two reads. It is the same as the sensor's measurement rate.
const POLL_INTERVAL: Duration = Duration::from_millis(100);
/// Raise the gain when the larger channel count is below this.
const GAIN_UP_BELOW: u16 = 500;
/// Lower the gain when the larger channel count is above this. It is close
/// to 65535, the largest count.
const GAIN_DOWN_ABOVE: u16 = 60000;
/// Index into `ALS_GAIN_CODES` of the gain to start with: 8x, in the middle
/// of the range.
const INITIAL_GAIN_INDEX: usize = 3;
/// Number of readings with a different gain that the task accepts after a
/// gain change. These readings may still have the previous gain. When more
/// readings show a different gain, the sensor has probably lost its
/// configuration, and the task configures it again. (For example, a
/// brown-out, a short drop of the supply voltage, resets it to standby at
/// 1x.)
const SETTLE_READINGS: u8 = 2;
/// Time between attempts to configure a sensor that does not answer.
const CONFIGURE_RETRY: Duration = Duration::from_secs(1);
/// Readings in a row with invalid light data after which the task configures
/// the sensor again: one second. After a gain change the data is invalid for
/// only one or two readings. Much longer means that the sensor has probably
/// lost its configuration, and then its proximity data is not fresh either.
const MAX_INVALID_READINGS: u8 = 10;

/// Start polling the sensor on CPU1. The task publishes to both handles.
///
/// # Panics
///
/// When the task is already running.
pub(crate) fn spawn(
    spawner: &Spawner,
    bus: SystemI2cBus,
    light: Runtime,
    proximity: proximity::Runtime,
) {
    spawner.spawn(poll_task(bus, light, proximity).expect("light task already spawned"));
}

/// Configure the sensor, then read and publish a sample every 100 ms.
///
/// A failed read is skipped. Only the first failure in a series is logged.
/// So a sensor that stops answering does not fill the log. After a
/// successful read, the next failure is logged again.
#[embassy_executor::task]
async fn poll_task(bus: SystemI2cBus, light: Runtime, proximity: proximity::Runtime) {
    let mut gain_index = INITIAL_GAIN_INDEX;
    configure_until_it_works(bus, gain_index).await;

    let mut read_failed = false;
    let mut reset_logged = false;
    // How many more readings may still show the previous gain after a change.
    let mut settling = SETTLE_READINGS;
    // Readings in a row without valid light data.
    let mut invalid_readings: u8 = 0;
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
                continue;
            }
        };

        proximity.publish(proximity_sample(reading));
        // The light data is invalid for a short time after a gain change.
        // The proximity sample above is published anyway, because the gain
        // change does not affect it.
        let Some(measurement) = reading.light else {
            invalid_readings = invalid_readings.saturating_add(1);
            if invalid_readings >= MAX_INVALID_READINGS {
                // Logged once, like the gain mismatch below, so a sensor that
                // never recovers does not fill the log.
                if !reset_logged {
                    warn!("LTR-553 light data stays invalid: configuring it again");
                    reset_logged = true;
                }
                configure_until_it_works(bus, gain_index).await;
                invalid_readings = 0;
                settling = SETTLE_READINGS;
            }
            continue;
        };
        invalid_readings = 0;
        light.publish(light_sample(measurement));

        // Choose a new gain only from a measurement at the requested gain.
        // The first readings after a change may still have the old gain.
        // A reported gain that is not in the table is ignored.
        let Some(reported) = gain_index_of(measurement.gain) else {
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
                    measurement.gain,
                    ltr553::ALS_GAIN_FACTORS[gain_index]
                );
                reset_logged = true;
            }
            configure_until_it_works(bus, gain_index).await;
            settling = SETTLE_READINGS;
            continue;
        }
        if let Some(next) = better_gain(gain_index, measurement.channels) {
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

/// Configure the sensor. When that fails, try again every second until it
/// works. The check during bring-up found the chip, so a failure here is
/// probably temporary. Only the first failure is logged.
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

/// The index of a gain factor that the sensor reports. The index is the
/// same for `ALS_GAIN_FACTORS` and `ALS_GAIN_CODES`. `None` for a factor
/// that is not in the table.
fn gain_index_of(factor: u8) -> Option<usize> {
    ltr553::ALS_GAIN_FACTORS.iter().position(|&f| f == factor)
}

/// The next gain index, when the counts need a gain change. One step down
/// when either channel is close to saturating. One step up when both
/// channels are small. `None` when the gain is right, or when it is already
/// at the end of the table.
///
/// The thresholds are far apart. So after one change, the new counts never
/// ask for the opposite change.
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

/// Convert one light measurement to lux. The conversion uses both channels
/// and the gain of this measurement.
fn light_sample(measurement: light::Light) -> Sample {
    Sample {
        lux: light::lux(
            measurement.channels,
            measurement.gain,
            ltr553::ALS_INTEGRATION_MS,
        ),
    }
}

/// The proximity part of one reading. `raw` is the count, or the maximum
/// count when the measurement is saturated. `percent` is the closeness,
/// spread evenly over the distance.
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
