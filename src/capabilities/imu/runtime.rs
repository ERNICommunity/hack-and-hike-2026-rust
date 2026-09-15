//! The CPU1 task that reads the IMU and runs the sensor fusion.
//!
//! Every 10 ms, the task does these steps:
//!
//! 1. Read one sample in a single I2C transaction. The sample has the
//!    acceleration, the rotation speed, the magnetometer frame and the
//!    BMI270's own timestamp.
//! 2. Remove the gyroscope offset (bias).
//! 3. Update the magnetometer calibration.
//! 4. Combine all measurements into one orientation (sensor fusion).
//!
//! When ten reads in a row fail, the task sets up the sensor again. The
//! magnetometer calibration learned so far stays.

use embassy_executor::Spawner;
use embassy_time::{Duration, Instant, Ticker, Timer};
use log::{info, trace, warn};

use crate::board::i2c::SystemI2cBus;

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

/// Samples read per second. The fusion runs once for each sample.
const SAMPLE_HZ: u32 = 100;
/// Time between two reads: 10 ms.
const SAMPLE_PERIOD: Duration = Duration::from_hz(SAMPLE_HZ as u64);
/// Rate of the BMI270's timestamp. The timestamp is a 24-bit counter that
/// counts up 25,600 times per second and never stops.
const SENSOR_TIME_HZ: u32 = 25_600;
/// Seconds per timestamp tick (about 39 µs).
const SENSOR_TIME_TICK_SECONDS: f32 = 1.0 / SENSOR_TIME_HZ as f32;
/// The counter has 24 bits. After its highest value, it starts again at 0.
const SENSOR_TIME_MASK: u32 = 0x00FF_FFFF;
/// Timestamp ticks between two samples when no sample is lost: 256.
const NOMINAL_SAMPLE_TICKS: u32 = SENSOR_TIME_HZ / SAMPLE_HZ;
/// Largest normal time between two samples: five sample periods (50 ms).
///
/// A longer time, or no time at all, means a gap: samples were lost. For a
/// gap, the integration uses one normal step instead of the measured time.
const MAX_SAMPLE_GAP_TICKS: u32 = NOMINAL_SAMPLE_TICKS * 5;
/// 1 g in m/s². The sample gives acceleration in m/s², not in g.
const STANDARD_GRAVITY_M_S2: f32 = 9.80665;

/// Wait after a failed setup before the next try.
const INIT_RETRY: Duration = Duration::from_secs(1);
/// Wait after too many failed reads before the sensor is set up again.
const REINIT_DELAY: Duration = Duration::from_millis(250);
/// Failed reads in a row before the sensor is set up again.
const MAX_CONSECUTIVE_READ_ERRORS: u8 = 10;
/// Gyroscope limit in degrees per second (dps). The range ends at 2000 dps.
/// Near this end, the gyroscope cannot show faster rotation. So the
/// integrated angle is not reliable.
const GYRO_NEAR_SATURATION_DPS: f32 = 1950.0;
/// Write a trace line for every 20th sample: five lines per second.
const TRACE_EVERY_SAMPLES: u32 = 20;
/// Shortest time between two warnings about an interrupted integration.
///
/// A fast turn of the board can saturate the gyroscope for many samples in a
/// row. A warning for each sample would slow the loop down. That would cause
/// new sample gaps.
const INTERRUPTION_LOG_INTERVAL: Duration = Duration::from_secs(1);

/// Start the IMU task on CPU1.
///
/// # Panics
///
/// When the IMU task is already running.
pub(crate) fn spawn(spawner: &Spawner, bus: SystemI2cBus, runtime: Runtime) {
    spawner.spawn(capture_task(bus, runtime).expect("IMU task already spawned"));
}

/// Set up the sensors, then read and fuse samples forever.
///
/// The outer loop sets up the sensors, and again after a failure. The inner
/// loop runs once per sample. When the BMM150 magnetometer fails to start,
/// the task continues with the accelerometer and the gyroscope only.
#[embassy_executor::task]
async fn capture_task(bus: SystemI2cBus, runtime: Runtime) {
    let sensor = Bmi270::new(bus);
    let mut publisher = Publisher::new(runtime);
    // The calibration describes the sensor and the board around it, not one
    // session. So it stays when the sensor is set up again. The last
    // orientation also stays, so that `Fault` and `Degraded` samples can
    // repeat it.
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
            // Read the clock after the wait, so that `now` is the time of
            // this sample.
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

/// Gives each sample a number and sends it to the application.
struct Publisher {
    /// Where the samples go.
    runtime: Runtime,
    /// Number of the last published sample. After `u32::MAX`, it starts again
    /// at 0.
    revision: u32,
}

impl Publisher {
    /// A publisher that has not published anything yet. The first sample
    /// gets revision 1.
    const fn new(runtime: Runtime) -> Self {
        Self {
            runtime,
            revision: 0,
        }
    }

    /// Convert the measurements to the screen frame, calculate the attitude
    /// and publish everything as the next [`Sample`].
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

/// The fusion state for one session. A session starts when the sensor is set
/// up and ends when it must be set up again.
struct Session {
    /// The sensor fusion. It combines rotation speed, gravity and the
    /// magnetic field into one orientation.
    fusion: Fusion,
    /// Learns the gyroscope's zero offset while the board lies still.
    gyro_bias: GyroBias,
    /// The sensor timestamp of the previous sample. It gives the real time
    /// between two samples. `None` before the first sample.
    last_sensor_time: Option<u32>,
    /// Failed reads since the last successful read.
    consecutive_read_errors: u8,
    /// Samples processed in this session. With this count, the task writes
    /// one trace line per `TRACE_EVERY_SAMPLES` samples.
    samples: u32,
    /// The magnetometer status that was logged last. Only changes are
    /// logged.
    mag_status: MagStatus,
    /// When the last warning about an interrupted integration was logged.
    /// `None` before the first warning.
    interruption_logged_at: Option<Instant>,
    /// Interruptions since the last warning that got no warning of their own.
    interruptions_not_logged: u32,
}

impl Session {
    /// A new session: no timestamp yet, no errors, and the gyroscope offset
    /// is learned again from zero. `mag_status` is the current magnetometer
    /// status, so that only later changes are logged.
    const fn new(mag_status: MagStatus) -> Self {
        Self {
            fusion: Fusion::new(),
            gyro_bias: GyroBias::new(),
            last_sensor_time: None,
            consecutive_read_errors: 0,
            samples: 0,
            mag_status,
            interruption_logged_at: None,
            interruptions_not_logged: 0,
        }
    }

    /// The time step for the integration, in seconds, and whether there was
    /// a gap.
    ///
    /// The step is the time since the previous sample, measured with the
    /// sensor's own clock. For the first sample, and after a gap, the step is
    /// one normal sample period. A gap means that the timestamp did not
    /// change, or that more than `MAX_SAMPLE_GAP_TICKS` ticks passed.
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

    /// Process one successful read: measure the time step, remove the
    /// gyroscope offset, update the magnetometer state and run the fusion.
    ///
    /// After a sample gap or a saturated gyroscope, the fusion forgets the
    /// heading from the magnetometer and finds it again. After a gap, it
    /// also forgets the previous rotation speed.
    ///
    /// Returns the physical measurements (body frame) and the new orientation.
    /// The measured rotation speed is the value before the offset is removed.
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
            self.log_interruption(gap, saturated, now);
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

    /// Log a warning about an interrupted integration.
    ///
    /// There is at most one warning per `INTERRUPTION_LOG_INTERVAL`. The
    /// interruptions in between are only counted. The next warning shows
    /// this count.
    fn log_interruption(&mut self, gap: bool, saturated: bool, now: Instant) {
        let due = self
            .interruption_logged_at
            .is_none_or(|logged_at| now - logged_at >= INTERRUPTION_LOG_INTERVAL);
        if !due {
            self.interruptions_not_logged = self.interruptions_not_logged.saturating_add(1);
            return;
        }
        warn!(
            "IMU integration interrupted (sample gap: {gap}, gyro saturated: {saturated}; {} earlier interruptions not logged); heading will be re-acquired",
            self.interruptions_not_logged
        );
        self.interruption_logged_at = Some(now);
        self.interruptions_not_logged = 0;
    }

    /// Count a failed read. Returns `true` when `MAX_CONSECUTIVE_READ_ERRORS`
    /// reads in a row failed, so the sensor must be set up again.
    fn record_read_error(&mut self) -> bool {
        self.consecutive_read_errors = self.consecutive_read_errors.saturating_add(1);
        self.consecutive_read_errors >= MAX_CONSECUTIVE_READ_ERRORS
    }
}
