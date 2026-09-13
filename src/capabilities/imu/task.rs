//! CPU1 IMU acquisition loop.

use embassy_time::{Duration, Instant, Ticker, Timer};
use log::{info, trace, warn};

use crate::platform::i2c::SystemI2cBus;

use hack_and_hike_core::imu::{
    Orientation,
    fusion::{Fusion, GyroBias},
    vec3,
};

use super::{
    MagStatus, Measurements, Status,
    bmi270::{Bmi270, RawSample},
    channels::{Publisher, Runtime},
    magnetic::MagneticState,
};

/// Host sampling rate; fusion runs once per sample.
const SAMPLE_HZ: u32 = 100;
const SAMPLE_PERIOD: Duration = Duration::from_hz(SAMPLE_HZ as u64);
// BMI270 sensor time is a free-running 24-bit counter at 25.6 kHz.
const SENSOR_TIME_HZ: u32 = 25_600;
const SENSOR_TIME_TICK_SECONDS: f32 = 1.0 / SENSOR_TIME_HZ as f32;
const SENSOR_TIME_MASK: u32 = 0x00FF_FFFF;
const NOMINAL_SAMPLE_TICKS: u32 = SENSOR_TIME_HZ / SAMPLE_HZ;
/// A longer gap between samples means some were lost; the integration then
/// uses one nominal step instead of the measured interval.
const MAX_SAMPLE_GAP_TICKS: u32 = NOMINAL_SAMPLE_TICKS * 5;
const STANDARD_GRAVITY_M_S2: f32 = 9.80665;

const INIT_RETRY: Duration = Duration::from_secs(1);
const REINIT_DELAY: Duration = Duration::from_millis(250);
const MAX_CONSECUTIVE_READ_ERRORS: u8 = 10;
/// Near full scale the gyroscope clips, so integration cannot be trusted.
const GYRO_NEAR_SATURATION_DPS: f32 = 1950.0;
/// Trace every 20th sample: five lines per second.
const TRACE_EVERY_SAMPLES: u32 = 20;

#[embassy_executor::task]
pub(crate) async fn capture_task(bus: SystemI2cBus, runtime: Runtime) {
    let sensor = Bmi270::new(bus);
    let mut publisher = Publisher::new(runtime);
    // Calibration describes the physical sensor and enclosure, not one
    // session, so it survives sensor re-initialization.
    let mut magnetic = MagneticState::new(Instant::now());
    let mut orientation = Orientation::default();

    loop {
        publisher.publish(
            Status::Starting,
            Measurements::default(),
            orientation,
            magnetic.report(),
        );
        if let Err(error) = sensor.initialize().await {
            warn!("BMI270 init failed: {error}");
            publisher.publish(
                Status::Fault,
                Measurements::default(),
                orientation,
                magnetic.report(),
            );
            Timer::after(INIT_RETRY).await;
            continue;
        }

        let trim = match sensor.initialize_bmm150().await {
            Ok(trim) => Some(trim),
            Err(error) => {
                warn!("BMM150 init failed: {error}; heading will drift until it recovers");
                sensor.disable_aux().await;
                None
            }
        };
        magnetic.rebind(trim, Instant::now());
        info!("IMU started at {} Hz", SAMPLE_HZ);

        let mut session = Session::new(magnetic.status());
        let mut ticker = Ticker::every(SAMPLE_PERIOD);
        loop {
            ticker.next().await;
            let now = Instant::now();
            magnetic.maintain(&sensor, now).await;

            match sensor.read_sample().await {
                Ok(sample) => {
                    let measurements;
                    (measurements, orientation) = session.process(sample, &mut magnetic, now);
                    publisher.publish(
                        Status::Running,
                        measurements,
                        orientation,
                        magnetic.report(),
                    );
                }
                Err(error) => {
                    if session.record_read_error() {
                        warn!(
                            "BMI270 read failed {MAX_CONSECUTIVE_READ_ERRORS} times in a row ({error}); re-initializing"
                        );
                        publisher.publish(
                            Status::Fault,
                            Measurements::default(),
                            orientation,
                            magnetic.report(),
                        );
                        Timer::after(REINIT_DELAY).await;
                        break;
                    }
                    publisher.publish(
                        Status::Degraded,
                        Measurements::default(),
                        orientation,
                        magnetic.report(),
                    );
                }
            }
        }
    }
}

/// Fusion state for one sensor session, which lasts until the sensor is
/// re-initialized.
struct Session {
    fusion: Fusion,
    gyro_bias: GyroBias,
    last_sensor_time: Option<u32>,
    consecutive_read_errors: u8,
    samples: u32,
    mag_status: MagStatus,
}

impl Session {
    const fn new(mag_status: MagStatus) -> Self {
        Self {
            fusion: Fusion::new(),
            gyro_bias: GyroBias::new(),
            last_sensor_time: None,
            consecutive_read_errors: 0,
            samples: 0,
            mag_status,
        }
    }

    /// Seconds since the previous sample by the sensor's own clock, and
    /// whether samples were lost in between.
    fn integration_step(&mut self, sensor_time: u32) -> (f32, bool) {
        let delta_ticks = self
            .last_sensor_time
            .map_or(NOMINAL_SAMPLE_TICKS, |previous| {
                sensor_time.wrapping_sub(previous) & SENSOR_TIME_MASK
            });
        self.last_sensor_time = Some(sensor_time);

        let gap = delta_ticks == 0 || delta_ticks > MAX_SAMPLE_GAP_TICKS;
        let ticks = if gap {
            NOMINAL_SAMPLE_TICKS
        } else {
            delta_ticks
        };
        (ticks as f32 * SENSOR_TIME_TICK_SECONDS, gap)
    }

    fn process(
        &mut self,
        sample: RawSample,
        magnetic: &mut MagneticState,
        now: Instant,
    ) -> (Measurements, Orientation) {
        self.consecutive_read_errors = 0;
        self.samples = self.samples.wrapping_add(1);

        let (dt_seconds, gap) = self.integration_step(sample.sensor_time);
        let saturated = vec3::max_abs(sample.gyro_dps) >= GYRO_NEAR_SATURATION_DPS;
        if gap || saturated {
            warn!(
                "IMU integration interrupted (sample gap: {gap}, gyro saturated: {saturated}); heading will be re-acquired"
            );
            self.fusion.invalidate_absolute_heading();
        }
        if gap {
            self.fusion.reset_rate_history();
        }

        let gyro_dps = self.gyro_bias.correct(sample.accel_g, sample.gyro_dps);
        let magnetic_for_fusion = magnetic.observe(sample.mag_data, now);
        let orientation =
            self.fusion
                .update(sample.accel_g, gyro_dps, dt_seconds, magnetic_for_fusion);

        if magnetic.status() != self.mag_status {
            info!(
                "Magnetometer {:?} -> {:?}",
                self.mag_status,
                magnetic.status()
            );
            self.mag_status = magnetic.status();
        }
        if self.samples.is_multiple_of(TRACE_EVERY_SAMPLES) {
            trace!(
                "IMU dt={:.1}ms roll={:.1} pitch={:.1} yaw={:.1} mag={:?} cal={}%",
                dt_seconds * 1000.0,
                orientation.roll_deg,
                orientation.pitch_deg,
                orientation.yaw_deg,
                magnetic.status(),
                magnetic.report().calibration_percent
            );
        }

        let measurements = Measurements {
            acceleration_m_s2: Some(vec3::scale(sample.accel_g, STANDARD_GRAVITY_M_S2)),
            angular_velocity_deg_s: Some(sample.gyro_dps),
            magnetic_field_ut: magnetic.vector_ut(),
        };
        (measurements, orientation)
    }

    /// Count a failed read. Returns true once the sensor should be
    /// re-initialized.
    fn record_read_error(&mut self) -> bool {
        self.consecutive_read_errors = self.consecutive_read_errors.saturating_add(1);
        self.consecutive_read_errors >= MAX_CONSECUTIVE_READ_ERRORS
    }
}
