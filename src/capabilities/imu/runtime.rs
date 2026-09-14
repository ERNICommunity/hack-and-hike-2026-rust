//! CPU1 task reading the IMU and running the sensor fusion.
//!
//! Every 10 ms: read one sample (acceleration, rotation, the magnetometer
//! frame and the sensor's own timestamp) in a single I2C transaction, correct
//! the gyroscope bias, update the magnetometer calibration and fuse
//! everything into an orientation. A sensor that stops answering is
//! re-initialized; the calibration learned so far survives that.

use embassy_executor::Spawner;
use embassy_time::{Duration, Instant, Ticker, Timer};
use log::{info, trace, warn};

use crate::platform::i2c::SystemI2cBus;

use hack_and_hike_core::imu::{
    Orientation,
    fusion::{Fusion, GyroBias},
    vec3,
};

use super::{
    Attitude, MagStatus, Measurements, Runtime, Sample, Status,
    bmi270::{Bmi270, RawSample},
    magnetic::{MagneticReport, MagneticState},
};

/// Samples read per second; fusion runs once per sample.
const SAMPLE_HZ: u32 = 100;
/// Time between two reads.
const SAMPLE_PERIOD: Duration = Duration::from_hz(SAMPLE_HZ as u64);
/// The BMI270's timestamp is a free-running 24-bit counter at 25.6 kHz.
const SENSOR_TIME_HZ: u32 = 25_600;
/// Seconds per timestamp tick.
const SENSOR_TIME_TICK_SECONDS: f32 = 1.0 / SENSOR_TIME_HZ as f32;
/// The counter wraps at 24 bits.
const SENSOR_TIME_MASK: u32 = 0x00FF_FFFF;
/// Timestamp ticks between two samples when none is lost.
const NOMINAL_SAMPLE_TICKS: u32 = SENSOR_TIME_HZ / SAMPLE_HZ;
/// A longer gap between samples means some were lost; the integration then
/// uses one nominal step instead of the measured interval.
const MAX_SAMPLE_GAP_TICKS: u32 = NOMINAL_SAMPLE_TICKS * 5;
/// 1 g in m/s², to publish acceleration in SI units.
const STANDARD_GRAVITY_M_S2: f32 = 9.80665;

/// Wait after a failed initialization before trying again.
const INIT_RETRY: Duration = Duration::from_secs(1);
/// Wait before re-initializing a sensor that stopped answering.
const REINIT_DELAY: Duration = Duration::from_millis(250);
/// Failed reads in a row before the sensor is re-initialized.
const MAX_CONSECUTIVE_READ_ERRORS: u8 = 10;
/// Near full scale the gyroscope clips, so integration cannot be trusted.
const GYRO_NEAR_SATURATION_DPS: f32 = 1950.0;
/// Trace every 20th sample: five lines per second.
const TRACE_EVERY_SAMPLES: u32 = 20;

/// Start IMU acquisition on CPU1.
pub(crate) fn spawn(spawner: &Spawner, bus: SystemI2cBus, runtime: Runtime) {
    spawner.spawn(capture_task(bus, runtime).expect("IMU task already spawned"));
}

/// Initialize the sensors, then read and fuse samples forever. The outer loop
/// re-initializes after a failure; the inner loop runs once per sample.
#[embassy_executor::task]
async fn capture_task(bus: SystemI2cBus, runtime: Runtime) {
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

/// Numbers the samples and hands them to the application side.
struct Publisher {
    /// Where samples go.
    runtime: Runtime,
    /// Number of the last published sample; wraps around at `u32::MAX`.
    revision: u32,
}

impl Publisher {
    /// A publisher that has not published anything yet.
    const fn new(runtime: Runtime) -> Self {
        Self {
            runtime,
            revision: 0,
        }
    }

    /// Convert the measurements to the screen frame, derive the attitude and
    /// publish everything as the next [`Sample`].
    fn publish(
        &mut self,
        status: Status,
        measurements: Measurements,
        orientation: Orientation,
        magnetic: MagneticReport,
    ) {
        self.revision = self.revision.wrapping_add(1);
        let measurements = measurements.in_screen_frame();
        self.runtime.publish(Sample {
            revision: self.revision,
            status,
            attitude: Attitude::from_orientation(&orientation),
            acceleration_m_s2: measurements.acceleration_m_s2,
            angular_velocity_deg_s: measurements.angular_velocity_deg_s,
            magnetic_field_ut: measurements.magnetic_field_ut,
            mag_status: magnetic.status,
            mag_field_strength_ut: magnetic.field_ut,
            mag_calibration_percent: magnetic.calibration_percent,
        });
    }
}

/// Fusion state for one sensor session, which lasts until the sensor is
/// re-initialized.
struct Session {
    /// Sensor fusion that turns rates, gravity and the magnetic field into an
    /// orientation.
    fusion: Fusion,
    /// Learns the gyroscope's zero offset while the board lies still.
    gyro_bias: GyroBias,
    /// The sensor timestamp of the previous sample, to measure the real time
    /// step; `None` before the first sample.
    last_sensor_time: Option<u32>,
    /// Failed reads since the last successful one.
    consecutive_read_errors: u8,
    /// Samples processed in this session, used to thin out trace logging.
    samples: u32,
    /// Magnetometer status last logged, to log only changes.
    mag_status: MagStatus,
}

impl Session {
    /// A fresh session: no timestamp yet, no errors, bias learning from zero.
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

    /// Process one successful read: measure the time step, correct the gyro
    /// bias, update the magnetometer and run fusion.
    ///
    /// Returns the physical measurements (body frame) and the new orientation.
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
