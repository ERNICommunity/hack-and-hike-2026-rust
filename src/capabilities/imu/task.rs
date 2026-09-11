//! Runtime IMU acquisition and service orchestration.

use embassy_time::{Duration, Instant, Ticker, Timer};

use crate::platform::i2c::SystemI2cBus;

use super::{
    Config, DEFAULT_FUSION_HZ, DEFAULT_MAG_HZ, DEFAULT_SENSOR_HZ, MagStatus, Orientation, Status,
    bmi270::{Bmi270, Error, GYRO_SENSOR_ODR_HZ},
    channels::{self, Runtime},
    fusion::{Fusion, GyroBias, max_abs3},
    magnetic::MagneticState,
};

// BMI270 sensor time is a free-running 24-bit counter at exactly 25.6 kHz.
const SENSOR_TIME_TICK_SECONDS: f32 = 1.0 / 25_600.0;
const SENSOR_TIME_MASK: u32 = 0x00FF_FFFF;
const MAX_FUSION_SAMPLE_GAP_TICKS: u32 = 1_280; // 50 ms
const NOMINAL_FUSION_TICKS: u32 = 25_600 / DEFAULT_SENSOR_HZ; // 10 ms

const INIT_RETRY: Duration = Duration::from_secs(1);
const MAX_CONSECUTIVE_READ_ERRORS: u8 = 10;
// If a trusted gyro integration is known to have become incomplete (near full
// scale or a long sample gap), explicitly mark absolute yaw untrusted.
const GYRO_NEAR_SATURATION_DPS: f32 = 1950.0;

// The steady-state trace is intentionally much slower than the 100 Hz fusion
// loop. Five lines per second is dense enough to reconstruct motion while not
// making serial logging itself a meaningful source of sample jitter.
const IMU_TRACE_EVERY_SAMPLES: u32 = 20;

#[embassy_executor::task]
pub(crate) async fn capture_task(bus: SystemI2cBus, config: Config, runtime: Runtime) {
    let sensor = Bmi270::new(bus);
    let mut revision = 0u32;
    let mut last_orientation = Orientation::default();
    let mut logged_calibrated_publish = false;
    // Calibration describes the physical sensor/enclosure, not one transport
    // session. Keep it alive across BMI270/AUX recovery for the whole boot.
    let mut magnetic = MagneticState::new(Instant::now());

    loop {
        magnetic.rebind(None, Instant::now());
        channels::publish(
            runtime,
            &mut revision,
            Status::Starting,
            last_orientation,
            magnetic.status(),
            magnetic.field_ut(),
            magnetic.calibration_percent(),
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
                    magnetic.status(),
                    magnetic.field_ut(),
                    magnetic.calibration_percent(),
                );
                Timer::after(INIT_RETRY).await;
                continue;
            }
        }

        let initial_mag_trim = match sensor.initialize_bmm150().await {
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
        let now = Instant::now();
        magnetic.rebind(initial_mag_trim, now);
        let mut last_sensor_time: Option<u32> = None;
        let mut consecutive_errors = 0u8;
        let mut trace_samples = 0u32;
        let mut previous_mag_status = magnetic.status();
        // Ticker advances against a fixed deadline. The old Timer::after loop
        // added I2C/fusion execution time to every nominal 10 ms period and ran
        // closer to 80-90 Hz in the captured trace.
        let mut sample_ticker = Ticker::every(config.sample_period);

        ::log::trace!("IMU session-start revision={}", revision);

        loop {
            sample_ticker.next().await;
            let now = Instant::now();
            magnetic.maintain(&sensor, now).await;

            match sensor.read_sample().await {
                Ok(sample) => {
                    consecutive_errors = 0;
                    trace_samples = trace_samples.wrapping_add(1);

                    let delta_ticks = last_sensor_time
                        .map(|previous| {
                            sample.sensor_time.wrapping_sub(previous) & SENSOR_TIME_MASK
                        })
                        .unwrap_or(NOMINAL_FUSION_TICKS);
                    last_sensor_time = Some(sample.sensor_time);
                    let timing_gap = delta_ticks == 0 || delta_ticks > MAX_FUSION_SAMPLE_GAP_TICKS;
                    let integration_ticks = if timing_gap {
                        NOMINAL_FUSION_TICKS
                    } else {
                        delta_ticks
                    };
                    let dt_seconds = integration_ticks as f32 * SENSOR_TIME_TICK_SECONDS;
                    let raw_gyro_max = max_abs3(sample.gyro_dps);
                    let near_saturation = raw_gyro_max >= GYRO_NEAR_SATURATION_DPS;

                    if timing_gap || near_saturation {
                        ::log::warn!(
                            "IMU-EVENT integration-invalid sensor_time={} delta_ticks={} dt_ms={} gyro_max_dps={} timing_gap={} saturation={}",
                            sample.sensor_time,
                            delta_ticks,
                            dt_seconds * 1000.0,
                            raw_gyro_max,
                            timing_gap,
                            near_saturation
                        );
                        fusion.invalidate_absolute_heading();
                    }
                    if timing_gap {
                        fusion.reset_rate_history();
                    }

                    let corrected_gyro = gyro_bias.correct(sample.accel_g, sample.gyro_dps);
                    let magnetic_for_fusion = magnetic.observe(sample.mag_data, now);

                    last_orientation = fusion.update(
                        sample.accel_g,
                        corrected_gyro,
                        dt_seconds,
                        config.roll_pitch_alpha,
                        magnetic_for_fusion,
                        config.yaw_alpha,
                    );

                    let mag_status = magnetic.status();
                    let calibration_percent = magnetic.calibration_percent();
                    let status = match mag_status {
                        MagStatus::Learning => Status::Starting,
                        MagStatus::Ready => Status::Running,
                        MagStatus::Missing | MagStatus::Disturbed => Status::Degraded,
                    };

                    if mag_status != previous_mag_status {
                        ::log::warn!(
                            "IMU-EVENT mag-status {:?}->{:?} field_ut={} cal={} revision={}",
                            previous_mag_status,
                            mag_status,
                            magnetic.field_ut(),
                            calibration_percent,
                            revision.wrapping_add(1)
                        );
                        previous_mag_status = mag_status;
                    }

                    if trace_samples % IMU_TRACE_EVERY_SAMPLES == 0 {
                        let mag = magnetic_for_fusion.unwrap_or([0.0, 0.0, 0.0]);
                        ::log::trace!(
                            "IMU rev={} st={} dt_ms={} acc=[{},{},{}] gyro_raw=[{},{},{}] gyro_corr=[{},{},{}] mag_used={} mag=[{},{},{}] field_ut={} mag_status={:?} cal={} out_rpy=[{},{},{}] g=[{},{},{}] n=[{},{},{}]",
                            revision.wrapping_add(1),
                            sample.sensor_time,
                            dt_seconds * 1000.0,
                            sample.accel_g[0],
                            sample.accel_g[1],
                            sample.accel_g[2],
                            sample.gyro_dps[0],
                            sample.gyro_dps[1],
                            sample.gyro_dps[2],
                            corrected_gyro[0],
                            corrected_gyro[1],
                            corrected_gyro[2],
                            magnetic_for_fusion.is_some(),
                            mag[0],
                            mag[1],
                            mag[2],
                            magnetic.field_ut(),
                            mag_status,
                            calibration_percent,
                            last_orientation.roll_deg,
                            last_orientation.pitch_deg,
                            last_orientation.yaw_deg,
                            last_orientation.gravity_screen[0],
                            last_orientation.gravity_screen[1],
                            last_orientation.gravity_screen[2],
                            last_orientation.north_screen[0],
                            last_orientation.north_screen[1],
                            last_orientation.north_screen[2]
                        );
                    }

                    if calibration_percent == 100 && !logged_calibrated_publish {
                        ::log::info!(
                            "CPU1 publishing calibrated IMU snapshot: status={:?} mag_status={:?} field={}uT revision={}",
                            status,
                            mag_status,
                            magnetic.field_ut(),
                            revision.wrapping_add(1)
                        );
                        logged_calibrated_publish = true;
                    }
                    channels::publish(
                        runtime,
                        &mut revision,
                        status,
                        last_orientation,
                        mag_status,
                        magnetic.field_ut(),
                        calibration_percent,
                    );
                }
                Err(_) => {
                    consecutive_errors = consecutive_errors.saturating_add(1);
                    ::log::warn!(
                        "IMU-EVENT read-error consecutive={} revision={}",
                        consecutive_errors,
                        revision
                    );
                    channels::publish(
                        runtime,
                        &mut revision,
                        Status::Degraded,
                        last_orientation,
                        magnetic.status(),
                        magnetic.field_ut(),
                        magnetic.calibration_percent(),
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
                            magnetic.status(),
                            magnetic.field_ut(),
                            magnetic.calibration_percent(),
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
