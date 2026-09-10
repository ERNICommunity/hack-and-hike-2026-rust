//! Runtime IMU acquisition and service orchestration.

use embassy_time::{Duration, Instant, Timer};

use crate::system_i2c::SystemI2cBus;

use super::{
    Config, DEFAULT_FUSION_HZ, DEFAULT_MAG_HZ, DEFAULT_SENSOR_HZ, MagStatus, Orientation, Status,
    bmi270::{Bmi270, Error, GYRO_SENSOR_ODR_HZ},
    bmm150,
    channels::{self, Runtime},
    fusion::{Fusion, GyroBias, max_abs3},
};

// BMI270 sensor time is a free-running 24-bit counter at exactly 25.6 kHz.
const SENSOR_TIME_TICK_SECONDS: f32 = 1.0 / 25_600.0;
const SENSOR_TIME_MASK: u32 = 0x00FF_FFFF;
const MAX_FUSION_SAMPLE_GAP_TICKS: u32 = 1_280; // 50 ms
const NOMINAL_FUSION_TICKS: u32 = 25_600 / DEFAULT_SENSOR_HZ; // 10 ms

const INIT_RETRY: Duration = Duration::from_secs(1);
const MAG_RETRY: Duration = Duration::from_secs(5);
const MAG_STALE: Duration = Duration::from_secs(1);
const MAX_CONSECUTIVE_READ_ERRORS: u8 = 10;
// Hard-iron offsets inside the CoreS3 enclosure can approach the BMM150's own
// measurement limits. The sensor is specified around +/-1300 uT on X/Y and
// +/-2500 uT on Z, so a valid 3-D vector can exceed 2000 uT in magnitude. Learn
// those raw offsets first; only the calibrated vector is judged as geomagnetic.
const MAG_LEARNING_MIN_UT: f32 = 5.0;
const MAG_LEARNING_MAX_UT: f32 = 4000.0;
// Recover quickly after good magnetic data returns, but require roughly one
// second of consecutive bad 30 Hz samples before declaring the magnetometer
// disturbed. Short magnitude dips should not make the whole IMU status flap.
const MAG_GOOD_SAMPLES_TO_READY: u8 = 8;
const MAG_BAD_SAMPLES_TO_DISTURBED: u8 = 30;
// If a trusted gyro integration is known to have become incomplete (near full
// scale or a long sample gap), explicitly mark absolute yaw untrusted.
const GYRO_NEAR_SATURATION_DPS: f32 = 1950.0;

#[embassy_executor::task]
pub async fn capture_task(bus: SystemI2cBus, config: Config, runtime: Runtime) {
    let sensor = Bmi270::new(bus);
    let mut revision = 0u32;
    let mut last_orientation = Orientation::default();

    loop {
        channels::publish(
            runtime,
            &mut revision,
            Status::Starting,
            last_orientation,
            MagStatus::Missing,
            0.0,
            0,
        );

        match sensor.initialize().await {
            Ok(()) => {}
            Err(error) => {
                log_init_error("BMI270", error);
                channels::publish(
                    runtime,
                    &mut revision,
                    Status::Fault,
                    last_orientation,
                    MagStatus::Missing,
                    0.0,
                    0,
                );
                Timer::after(INIT_RETRY).await;
                continue;
            }
        }

        let mut mag_trim = match sensor.initialize_bmm150().await {
            Ok(trim) => {
                ::log::info!(
                    "BMI270+BMM150 IMU started: host={} Hz, fusion={} Hz, gyro={} Hz, mag={} Hz",
                    DEFAULT_SENSOR_HZ,
                    DEFAULT_FUSION_HZ,
                    GYRO_SENSOR_ODR_HZ,
                    DEFAULT_MAG_HZ
                );
                Some(trim)
            }
            Err(error) => {
                log_init_error("BMM150", error);
                sensor.disable_aux().await;
                ::log::warn!("IMU continuing in 6-axis fallback; BMM150 will retry");
                None
            }
        };

        let mut fusion = Fusion::new();
        let mut gyro_bias = GyroBias::new();
        let mut mag_calibration = bmm150::Calibration::new();
        let mut mag_status = if mag_trim.is_some() {
            MagStatus::Learning
        } else {
            MagStatus::Missing
        };
        let mut mag_field_ut = 0.0f32;
        let mut last_mag_frame: Option<[u8; 8]> = None;
        let mut last_mag_update = Instant::now();
        let mut last_mag_retry = Instant::now();
        let mut last_sensor_time: Option<u32> = None;
        let mut consecutive_errors = 0u8;
        let mut mag_good_samples = 0u8;
        let mut mag_bad_samples = 0u8;

        loop {
            Timer::after(config.sample_period).await;
            let now = Instant::now();

            if mag_trim.is_none() && now - last_mag_retry >= MAG_RETRY {
                last_mag_retry = now;
                match sensor.initialize_bmm150().await {
                    Ok(trim) => {
                        ::log::info!("BMM150 recovered; 9-axis heading fusion enabled");
                        mag_trim = Some(trim);
                        mag_calibration = bmm150::Calibration::new();
                        mag_status = MagStatus::Learning;
                        mag_good_samples = 0;
                        mag_bad_samples = 0;
                        last_mag_frame = None;
                        last_mag_update = now;
                    }
                    Err(_) => {
                        sensor.disable_aux().await;
                    }
                }
            }

            match sensor.read_sample().await {
                Ok(sample) => {
                    consecutive_errors = 0;

                    let delta_ticks = last_sensor_time
                        .map(|previous| sample.sensor_time.wrapping_sub(previous) & SENSOR_TIME_MASK)
                        .unwrap_or(NOMINAL_FUSION_TICKS);
                    last_sensor_time = Some(sample.sensor_time);
                    let timing_gap = delta_ticks == 0 || delta_ticks > MAX_FUSION_SAMPLE_GAP_TICKS;
                    let integration_ticks = if timing_gap {
                        NOMINAL_FUSION_TICKS
                    } else {
                        delta_ticks
                    };
                    let dt_seconds = integration_ticks as f32 * SENSOR_TIME_TICK_SECONDS;

                    // Saturation or a timing discontinuity can lose turn angle.
                    // Mark absolute yaw untrusted, but retain the best gyro path;
                    // quiet-state MAG recovery below will establish north again.
                    if timing_gap || max_abs3(sample.gyro_dps) >= GYRO_NEAR_SATURATION_DPS {
                        fusion.invalidate_absolute_heading();
                    }
                    if timing_gap {
                        fusion.reset_rate_history();
                    }

                    let corrected_gyro = gyro_bias.correct(sample.accel_g, sample.gyro_dps);
                    let mut magnetic_for_fusion: Option<[f32; 3]> = None;

                    if let Some(trim) = mag_trim {
                        let is_new_frame = last_mag_frame != Some(sample.mag_data);
                        if is_new_frame {
                            last_mag_frame = Some(sample.mag_data);
                            if let Some(mag) = bmm150::compensate(sample.mag_data, trim) {
                                if mag.data_ready {
                                    last_mag_update = now;

                                    let body_field = [
                                        mag.field_ut[0],
                                        -mag.field_ut[1],
                                        -mag.field_ut[2],
                                    ];
                                    let raw_learnable = (MAG_LEARNING_MIN_UT..=MAG_LEARNING_MAX_UT)
                                        .contains(&mag.field_strength_ut);

                                    // Once calibration is accepted, freeze its model.
                                    // The calibrator itself balances 3-D coverage and
                                    // validates a candidate on fresh measurements.
                                    let calibration_ready_before = mag_calibration.is_ready();
                                    if !calibration_ready_before && raw_learnable {
                                        mag_calibration.observe(body_field);
                                    }

                                    let calibration_ready = mag_calibration.is_ready();
                                    let corrected_field = mag_calibration.apply(body_field);
                                    mag_field_ut = if calibration_ready {
                                        bmm150::vector_length(corrected_field)
                                    } else {
                                        mag.field_strength_ut
                                    };

                                    if !calibration_ready {
                                        magnetic_for_fusion = None;
                                        mag_good_samples = 0;
                                        if raw_learnable {
                                            mag_bad_samples = 0;
                                            mag_status = MagStatus::Learning;
                                        } else {
                                            mag_bad_samples = mag_bad_samples.saturating_add(1);
                                            if mag_bad_samples >= MAG_BAD_SAMPLES_TO_DISTURBED {
                                                mag_status = MagStatus::Disturbed;
                                            }
                                        }
                                    } else {
                                        let field_good = (bmm150::GOOD_FIELD_MIN_UT
                                            ..=bmm150::GOOD_FIELD_MAX_UT)
                                            .contains(&mag_field_ut);

                                        if field_good {
                                            mag_good_samples = mag_good_samples.saturating_add(1);
                                            mag_bad_samples = 0;
                                            // A newly calibrated or recovered magnetometer
                                            // must prove several consecutive good vectors
                                            // before it is allowed to define magnetic north.
                                            if mag_status == MagStatus::Ready
                                                || mag_good_samples >= MAG_GOOD_SAMPLES_TO_READY
                                            {
                                                mag_status = MagStatus::Ready;
                                                magnetic_for_fusion = Some(corrected_field);
                                            }
                                        } else {
                                            // Reject the individual bad vector immediately
                                            // from yaw fusion, but do not flap the whole IMU
                                            // to DEGRADED unless the mismatch persists.
                                            mag_bad_samples = mag_bad_samples.saturating_add(1);
                                            mag_good_samples = 0;
                                            if mag_bad_samples >= MAG_BAD_SAMPLES_TO_DISTURBED {
                                                mag_status = MagStatus::Disturbed;
                                            }
                                        }
                                    }
                                }
                            } else {
                                mag_bad_samples = mag_bad_samples.saturating_add(1);
                                mag_good_samples = 0;
                                if mag_bad_samples >= MAG_BAD_SAMPLES_TO_DISTURBED {
                                    mag_status = MagStatus::Disturbed;
                                }
                            }
                        }

                        if now - last_mag_update >= MAG_STALE {
                            mag_status = MagStatus::Missing;
                            mag_good_samples = 0;
                            mag_bad_samples = 0;
                            magnetic_for_fusion = None;
                        }
                    } else {
                        mag_status = MagStatus::Missing;
                        magnetic_for_fusion = None;
                    }

                    last_orientation = fusion.update(
                        sample.accel_g,
                        corrected_gyro,
                        dt_seconds,
                        config.roll_pitch_alpha,
                        magnetic_for_fusion,
                        config.yaw_alpha,
                    );

                    let status = match mag_status {
                        MagStatus::Missing | MagStatus::Disturbed => Status::Degraded,
                        MagStatus::Learning | MagStatus::Ready => Status::Running,
                    };
                    channels::publish(
                        runtime,
                        &mut revision,
                        status,
                        last_orientation,
                        mag_status,
                        mag_field_ut,
                        mag_calibration.progress_percent(),
                    );
                }
                Err(_) => {
                    consecutive_errors = consecutive_errors.saturating_add(1);
                    channels::publish(
                        runtime,
                        &mut revision,
                        Status::Degraded,
                        last_orientation,
                        mag_status,
                        mag_field_ut,
                        mag_calibration.progress_percent(),
                    );

                    if consecutive_errors >= MAX_CONSECUTIVE_READ_ERRORS {
                        ::log::warn!(
                            "BMI270 read failed {} times consecutively; reinitializing IMU",
                            consecutive_errors
                        );
                        channels::publish(
                            runtime,
                            &mut revision,
                            Status::Fault,
                            last_orientation,
                            mag_status,
                            mag_field_ut,
                            mag_calibration.progress_percent(),
                        );
                        Timer::after(Duration::from_millis(250)).await;
                        break;
                    }
                }
            }
        }
    }
}

fn log_init_error(device: &str, error: Error) {
    match error {
        Error::ChipId(id) => ::log::warn!("{} init failed: chip id=0x{:02x}", device, id),
        Error::BmmChipId(id) => ::log::warn!("{} init failed: chip id=0x{:02x}", device, id),
        Error::ConfigStatus(status) => {
            ::log::warn!("{} init failed: config status=0x{:02x}", device, status)
        }
        Error::AuxBusy => ::log::warn!("{} init failed: BMI270 AUX interface busy", device),
        Error::Bus => ::log::warn!("{} init failed: I2C error", device),
    }
}
