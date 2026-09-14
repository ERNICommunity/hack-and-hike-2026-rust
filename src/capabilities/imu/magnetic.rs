//! Magnetometer availability, calibration and health.
//!
//! Sensor transport stays in `bmi270`; this module owns the runtime policy
//! around magnetometer recovery, calibration, freshness and fusion eligibility.

use embassy_time::{Duration, Instant};
use log::info;

use hack_and_hike_core::imu::{bmm150, frames, vec3};

use super::{MagStatus, bmi270::Bmi270};

/// How often a missing magnetometer is probed again.
const RETRY_PERIOD: Duration = Duration::from_secs(5);
/// A magnetometer that delivers no new frame for this long counts as missing.
const STALE_AFTER: Duration = Duration::from_secs(1);
/// Recover quickly after good data returns, but require about one second of
/// consecutive bad 30 Hz samples before declaring a disturbance.
const GOOD_SAMPLES_TO_READY: u8 = 8;
const BAD_SAMPLES_TO_DISTURBED: u8 = 30;

/// Everything known about the magnetometer between two samples.
pub(super) struct MagneticState {
    /// Factory trim values; `None` while no magnetometer answers.
    trim: Option<bmm150::Trim>,
    /// Hard- and soft-iron calibration, learned while the board is moved.
    calibration: bmm150::Calibration,
    /// Health as published to the application.
    status: MagStatus,
    /// Strength of the latest field in µT, calibrated once possible.
    field_ut: f32,
    /// The latest uncalibrated field in the body frame.
    vector_ut: Option<[f32; 3]>,
    /// The raw frame seen last. The BMI270 repeats a frame until the 30 Hz
    /// magnetometer delivers a new one; repeats are skipped.
    last_frame: Option<[u8; 8]>,
    /// When the latest new frame arrived, to detect a stalled sensor.
    last_update: Instant,
    /// When a missing magnetometer was last probed.
    last_retry: Instant,
    /// Plausible fields in a row since the last implausible one.
    good_samples: u8,
    /// Implausible fields in a row since the last plausible one.
    bad_samples: u8,
}

/// Magnetometer health as published alongside every IMU sample.
#[derive(Clone, Copy, Debug)]
pub(super) struct MagneticReport {
    /// See [`MagStatus`].
    pub(super) status: MagStatus,
    /// Strength of the latest field in µT.
    pub(super) field_ut: f32,
    /// Calibration progress, 0 to 100.
    pub(super) calibration_percent: u8,
}

impl MagneticState {
    /// A magnetometer that is not attached yet.
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

    /// Attach a freshly initialized magnetometer (or none) without discarding
    /// the calibration learned earlier: the BMI270 may be re-initialized after
    /// bus errors, but the enclosure's magnetic distortion does not change.
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

    /// Retry a missing magnetometer without disturbing the running 6-axis path.
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

    /// Observe the newest BMI270 AUX frame. Returns a body-frame magnetic
    /// vector only when that sample is fresh, calibrated and trusted.
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

    /// Decode a new raw frame, feed the calibration and decide whether the
    /// field may steer the heading.
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
        // Expose the physical measurement whether or not calibration lets it
        // influence heading yet.
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
            // Keep this vector out of heading fusion, but only flag the whole
            // magnetometer as disturbed once the mismatch persists.
            self.record_bad_sample();
            return None;
        }

        self.good_samples = self.good_samples.saturating_add(1);
        self.bad_samples = 0;
        // A newly calibrated magnetometer must prove several consecutive good
        // vectors before it may define north; a rebind resumes immediately.
        if self.status == MagStatus::Ready || self.good_samples >= GOOD_SAMPLES_TO_READY {
            self.status = MagStatus::Ready;
            return Some(corrected);
        }
        None
    }

    /// Count an implausible field; enough in a row mark the magnetometer as
    /// disturbed.
    fn record_bad_sample(&mut self) {
        self.bad_samples = self.bad_samples.saturating_add(1);
        self.good_samples = 0;
        if self.bad_samples >= BAD_SAMPLES_TO_DISTURBED {
            self.status = MagStatus::Disturbed;
        }
    }

    /// The current health.
    pub(super) const fn status(&self) -> MagStatus {
        self.status
    }

    /// The latest uncalibrated field in the body frame.
    pub(super) const fn vector_ut(&self) -> Option<[f32; 3]> {
        self.vector_ut
    }

    /// Health, strength and calibration progress for the next sample.
    pub(super) fn report(&self) -> MagneticReport {
        MagneticReport {
            status: self.status,
            field_ut: self.field_ut,
            calibration_percent: self.calibration.progress_percent(),
        }
    }
}
