//! The state of the BMM150 magnetometer: setup, calibration and health.
//!
//! The register access is in the `bmi270` module. This module decides:
//!
//! - when to try the magnetometer setup again,
//! - which measurements go into the calibration,
//! - when a magnetometer counts as missing because it sends no new data,
//! - which measurements the sensor fusion may use for the heading.
//!
//! The field values are in microtesla (µT). The Earth's field is about 25 to
//! 65 µT.

use embassy_time::{Duration, Instant};
use log::info;

use hack_and_hike_core::imu::{bmm150, frames, vec3};

use super::{MagStatus, bmi270::Bmi270};

/// Time between two tries to set up a magnetometer whose setup failed.
const RETRY_PERIOD: Duration = Duration::from_secs(5);
/// A magnetometer that gives no new valid measurement for this long counts as
/// [`MagStatus::Missing`].
const STALE_AFTER: Duration = Duration::from_secs(1);
/// Good fields in a row before a newly calibrated or disturbed magnetometer
/// becomes [`MagStatus::Ready`]. At 30 measurements per second, this takes
/// about 0.3 seconds, so the heading comes back quickly.
const GOOD_SAMPLES_TO_READY: u8 = 8;
/// Bad fields in a row before the status becomes [`MagStatus::Disturbed`].
/// At 30 measurements per second, this takes about one second, so one bad
/// field does not change the status.
const BAD_SAMPLES_TO_DISTURBED: u8 = 30;

/// Everything this module knows about the magnetometer.
pub(super) struct MagneticState {
    /// Factory trim values: the correction values that Bosch stores in each
    /// BMM150. `None` while the BMM150 is not set up, because its setup
    /// failed.
    trim: Option<bmm150::Trim>,
    /// The calibration, learned while the board turns. It removes the
    /// constant offset (hard iron) and the distortion (soft iron) that the
    /// board adds to the field.
    calibration: bmm150::Calibration,
    /// The status that the application sees.
    status: MagStatus,
    /// Strength of the newest field in µT. It is calibrated when the
    /// calibration is ready.
    field_ut: f32,
    /// The newest field before calibration, in the body frame.
    vector_ut: Option<[f32; 3]>,
    /// The raw frame from the previous read. The BMM150 measures only 30
    /// times per second, so the BMI270 gives the same frame several times.
    /// These repeated frames are skipped.
    last_frame: Option<[u8; 8]>,
    /// When the newest valid measurement arrived. It shows when the
    /// magnetometer stops sending data.
    last_update: Instant,
    /// When the last setup try happened.
    last_retry: Instant,
    /// Good fields in a row since the last bad field.
    good_samples: u8,
    /// Bad fields in a row since the last good field.
    bad_samples: u8,
}

/// The magnetometer information that goes into every IMU sample.
#[derive(Clone, Copy, Debug)]
pub(super) struct MagneticReport {
    /// See [`MagStatus`].
    pub(super) status: MagStatus,
    /// Strength of the newest field in µT.
    pub(super) field_ut: f32,
    /// Calibration progress, 0 to 100.
    pub(super) calibration_percent: u8,
}

impl MagneticState {
    /// The state before any magnetometer is set up: status
    /// [`MagStatus::Missing`] and an empty calibration.
    pub(super) fn new(now: Instant) -> Self {
        Self {
            trim: None,
            calibration: bmm150::Calibration::new(),
            status: MagStatus::Missing,
            field_ut: 0.0,
            vector_ut: None,
            last_frame: None,
            last_update: now,
            last_retry: now,
            good_samples: 0,
            bad_samples: 0,
        }
    }

    /// Start to use a magnetometer that was just set up, or none when `trim`
    /// is `None`.
    ///
    /// The calibration learned earlier stays. The BMI270 can be set up again
    /// after bus errors, but the magnetic distortion of the board does not
    /// change. With a finished calibration, the status is
    /// [`MagStatus::Ready`] at once.
    pub(super) fn rebind(&mut self, trim: Option<bmm150::Trim>, now: Instant) {
        self.trim = trim;
        self.status = match trim {
            Some(_) if self.calibration.is_ready() => MagStatus::Ready,
            Some(_) => MagStatus::Learning,
            None => MagStatus::Missing,
        };
        self.field_ut = 0.0;
        self.vector_ut = None;
        self.last_frame = None;
        self.last_update = now;
        self.last_retry = now;
        self.good_samples = 0;
        self.bad_samples = 0;
    }

    /// Try to set up the BMM150 again, at most once per `RETRY_PERIOD`.
    ///
    /// This happens only while the setup has failed (no trim values). A
    /// magnetometer that was set up but then stopped sending data is not set
    /// up again here. The accelerometer and the gyroscope keep working
    /// during the try.
    pub(super) async fn maintain(&mut self, sensor: &Bmi270, now: Instant) {
        if self.trim.is_some() || now - self.last_retry < RETRY_PERIOD {
            return;
        }
        self.last_retry = now;
        match sensor.initialize_bmm150().await {
            Ok(trim) => {
                info!("BMM150 recovered; heading fusion enabled");
                self.rebind(Some(trim), now);
            }
            Err(_) => sensor.disable_aux().await,
        }
    }

    /// Process the magnetometer frame from the newest BMI270 read.
    ///
    /// The BMI270 reads this frame from the BMM150 through its auxiliary
    /// (AUX) interface. Returns the calibrated field in the body frame only
    /// when the frame is new, the calibration is ready and the field looks
    /// like the Earth's field. The sensor fusion uses only this returned
    /// field for the heading.
    pub(super) fn observe(&mut self, data: [u8; 8], now: Instant) -> Option<[f32; 3]> {
        let Some(trim) = self.trim else {
            self.status = MagStatus::Missing;
            self.vector_ut = None;
            return None;
        };

        let mut for_fusion = None;
        if self.last_frame != Some(data) {
            self.last_frame = Some(data);
            for_fusion = self.observe_new_frame(data, trim, now);
        }

        if now - self.last_update >= STALE_AFTER {
            self.status = MagStatus::Missing;
            self.vector_ut = None;
            self.good_samples = 0;
            self.bad_samples = 0;
            for_fusion = None;
        }
        for_fusion
    }

    /// Decode a new raw frame, give it to the calibration and decide whether
    /// the fusion may use the field for the heading.
    ///
    /// A frame that cannot be decoded counts as a bad field. A frame without
    /// the "data ready" flag is ignored.
    fn observe_new_frame(
        &mut self,
        data: [u8; 8],
        trim: bmm150::Trim,
        now: Instant,
    ) -> Option<[f32; 3]> {
        let Some(sample) = bmm150::compensate(data, trim) else {
            self.record_bad_sample();
            return None;
        };
        if !sample.data_ready {
            return None;
        }

        self.last_update = now;
        let body_field = frames::body_from_magnetometer(sample.field_ut);
        // Publish the measured field, also when the fusion cannot use it for
        // the heading yet.
        self.vector_ut = Some(body_field);

        let learnable = bmm150::Calibration::is_learnable(body_field);
        if !self.calibration.is_ready() && learnable {
            self.calibration.observe(body_field);
        }

        if !self.calibration.is_ready() {
            self.field_ut = vec3::norm(body_field);
            self.good_samples = 0;
            if learnable {
                self.bad_samples = 0;
                self.status = MagStatus::Learning;
            } else {
                self.record_bad_sample();
            }
            return None;
        }

        let corrected = self.calibration.apply(body_field);
        self.field_ut = vec3::norm(corrected);
        if !bmm150::Calibration::is_earth_field(self.field_ut) {
            // Do not use this field for the heading. But set the status to
            // `Disturbed` only when the field stays wrong for a while.
            self.record_bad_sample();
            return None;
        }

        self.good_samples = self.good_samples.saturating_add(1);
        self.bad_samples = 0;
        // A magnetometer that is not `Ready` yet needs several good fields in
        // a row before it may define north. A magnetometer that is already
        // `Ready` (for example after `rebind`) uses the field at once.
        if self.status == MagStatus::Ready || self.good_samples >= GOOD_SAMPLES_TO_READY {
            self.status = MagStatus::Ready;
            return Some(corrected);
        }
        None
    }

    /// Count a bad field. After `BAD_SAMPLES_TO_DISTURBED` bad fields in a
    /// row, the status becomes [`MagStatus::Disturbed`].
    fn record_bad_sample(&mut self) {
        self.bad_samples = self.bad_samples.saturating_add(1);
        self.good_samples = 0;
        if self.bad_samples >= BAD_SAMPLES_TO_DISTURBED {
            self.status = MagStatus::Disturbed;
        }
    }

    /// The current status.
    pub(super) const fn status(&self) -> MagStatus {
        self.status
    }

    /// The newest field before calibration, in µT, in the body frame.
    pub(super) const fn vector_ut(&self) -> Option<[f32; 3]> {
        self.vector_ut
    }

    /// Status, field strength and calibration progress for the next sample.
    pub(super) fn report(&self) -> MagneticReport {
        MagneticReport {
            status: self.status,
            field_ut: self.field_ut,
            calibration_percent: self.calibration.progress_percent(),
        }
    }
}
