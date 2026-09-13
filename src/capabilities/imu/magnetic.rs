//! Magnetometer availability, calibration and health.
//!
//! Sensor transport stays in `bmi270`; this module owns the runtime policy
//! around magnetometer recovery, calibration, freshness and fusion eligibility.

use embassy_time::{Duration, Instant};
use log::info;

use super::{MagStatus, bmi270::Bmi270, bmm150, frames, vec3};

/// How often a missing magnetometer is probed again.
const RETRY_PERIOD: Duration = Duration::from_secs(5);
/// A magnetometer that delivers no new frame for this long counts as missing.
const STALE_AFTER: Duration = Duration::from_secs(1);
/// Recover quickly after good data returns, but require about one second of
/// consecutive bad 30 Hz samples before declaring a disturbance.
const GOOD_SAMPLES_TO_READY: u8 = 8;
const BAD_SAMPLES_TO_DISTURBED: u8 = 30;

pub(super) struct MagneticState {
    trim: Option<bmm150::Trim>,
    calibration: bmm150::Calibration,
    status: MagStatus,
    field_ut: f32,
    vector_ut: Option<[f32; 3]>,
    last_frame: Option<[u8; 8]>,
    last_update: Instant,
    last_retry: Instant,
    good_samples: u8,
    bad_samples: u8,
}

/// Magnetometer health as published alongside every IMU sample.
#[derive(Clone, Copy, Debug)]
pub(super) struct MagneticReport {
    pub(super) status: MagStatus,
    pub(super) field_ut: f32,
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

    fn record_bad_sample(&mut self) {
        self.bad_samples = self.bad_samples.saturating_add(1);
        self.good_samples = 0;
        if self.bad_samples >= BAD_SAMPLES_TO_DISTURBED {
            self.status = MagStatus::Disturbed;
        }
    }

    pub(super) const fn status(&self) -> MagStatus {
        self.status
    }

    pub(super) const fn vector_ut(&self) -> Option<[f32; 3]> {
        self.vector_ut
    }

    pub(super) fn report(&self) -> MagneticReport {
        MagneticReport {
            status: self.status,
            field_ut: self.field_ut,
            calibration_percent: self.calibration.progress_percent(),
        }
    }
}
