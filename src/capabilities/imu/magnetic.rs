//! BMM150 runtime availability, calibration, and magnetic-health state.
//!
//! Sensor transport stays in `bmi270`; this module owns only the runtime policy
//! around magnetometer recovery, calibration, freshness, and fusion eligibility.

use embassy_time::{Duration, Instant};

use super::{MagStatus, bmi270::Bmi270, bmm150};

const MAG_RETRY: Duration = Duration::from_secs(5);
const MAG_STALE: Duration = Duration::from_secs(1);

// Hard-iron offsets inside the CoreS3 enclosure can approach the BMM150's own
// measurement limits. Learn those raw offsets first; only the calibrated vector
// is judged as geomagnetic.
const MAG_LEARNING_MIN_UT: f32 = 5.0;
const MAG_LEARNING_MAX_UT: f32 = 4000.0;

// Recover quickly after good magnetic data returns, but require roughly one
// second of consecutive bad 30 Hz samples before declaring a disturbance.
const MAG_GOOD_SAMPLES_TO_READY: u8 = 8;
const MAG_BAD_SAMPLES_TO_DISTURBED: u8 = 30;

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

impl MagneticState {
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

    /// Attach the currently initialized BMM150 transport state without throwing
    /// away a calibration model learned earlier in this boot. BMI270/AUX can be
    /// reinitialized after transient bus/read failures; enclosure hard/soft-iron
    /// calibration is still valid for the same physical magnetometer.
    pub(super) fn rebind(&mut self, trim: Option<bmm150::Trim>, now: Instant) {
        self.trim = trim;
        self.status = if trim.is_some() {
            if self.calibration.is_ready() {
                MagStatus::Ready
            } else {
                MagStatus::Learning
            }
        } else {
            MagStatus::Missing
        };
        self.field_ut = 0.0;
        self.vector_ut = None;
        self.last_frame = None;
        self.last_update = now;
        self.last_retry = now;
        self.good_samples = 0;
        self.bad_samples = 0;
    }

    /// Retry a missing BMM150 without disturbing the running 6-axis IMU path.
    pub(super) async fn maintain(&mut self, sensor: &Bmi270, now: Instant) {
        if self.trim.is_some() || now - self.last_retry < MAG_RETRY {
            return;
        }

        self.last_retry = now;
        match sensor.initialize_bmm150().await {
            Ok(trim) => {
                ::log::info!("BMM150 recovered; 9-axis heading fusion enabled");
                self.rebind(Some(trim), now);
            }
            Err(_) => sensor.disable_aux().await,
        }
    }

    /// Observe the newest BMI270 AUX frame and return a magnetic vector only
    /// when that individual BMM150 sample is fresh, calibrated, and trusted.
    pub(super) fn observe(&mut self, data: [u8; 8], now: Instant) -> Option<[f32; 3]> {
        let Some(trim) = self.trim else {
            self.status = MagStatus::Missing;
            self.vector_ut = None;
            return None;
        };

        let mut magnetic_for_fusion = None;
        if self.last_frame != Some(data) {
            self.last_frame = Some(data);
            self.observe_new_frame(data, trim, now, &mut magnetic_for_fusion);
        }

        if now - self.last_update >= MAG_STALE {
            self.status = MagStatus::Missing;
            self.vector_ut = None;
            self.good_samples = 0;
            self.bad_samples = 0;
            magnetic_for_fusion = None;
        }

        magnetic_for_fusion
    }

    fn observe_new_frame(
        &mut self,
        data: [u8; 8],
        trim: bmm150::Trim,
        now: Instant,
        magnetic_for_fusion: &mut Option<[f32; 3]>,
    ) {
        let Some(mag) = bmm150::compensate(data, trim) else {
            self.record_bad_sample();
            return;
        };
        if !mag.data_ready {
            return;
        }

        self.last_update = now;
        let body_field = [mag.field_ut[0], -mag.field_ut[1], -mag.field_ut[2]];
        // Expose the compensated physical body-frame measurement independently
        // of whether calibration currently allows it to influence yaw fusion.
        self.vector_ut = Some(body_field);
        let raw_learnable =
            (MAG_LEARNING_MIN_UT..=MAG_LEARNING_MAX_UT).contains(&mag.field_strength_ut);

        // Once calibration is accepted, freeze its model. The calibrator itself
        // balances 3-D coverage and validates a candidate on fresh measurements.
        if !self.calibration.is_ready() && raw_learnable {
            self.calibration.observe(body_field);
        }

        let calibration_ready = self.calibration.is_ready();
        let corrected_field = self.calibration.apply(body_field);
        self.field_ut = if calibration_ready {
            bmm150::vector_length(corrected_field)
        } else {
            mag.field_strength_ut
        };

        if !calibration_ready {
            self.good_samples = 0;
            if raw_learnable {
                self.bad_samples = 0;
                self.status = MagStatus::Learning;
            } else {
                self.record_bad_sample();
            }
            return;
        }

        let field_good =
            (bmm150::GOOD_FIELD_MIN_UT..=bmm150::GOOD_FIELD_MAX_UT).contains(&self.field_ut);
        if field_good {
            self.good_samples = self.good_samples.saturating_add(1);
            self.bad_samples = 0;
            // A newly calibrated magnetometer must prove several consecutive
            // good vectors before it may define north. A transport rebind keeps
            // the already-validated calibration and can resume immediately.
            if self.status == MagStatus::Ready || self.good_samples >= MAG_GOOD_SAMPLES_TO_READY {
                self.status = MagStatus::Ready;
                *magnetic_for_fusion = Some(corrected_field);
            }
        } else {
            // Reject the individual bad vector immediately from yaw fusion, but
            // do not flap the whole IMU to DEGRADED unless mismatch persists.
            self.record_bad_sample();
        }
    }

    fn record_bad_sample(&mut self) {
        self.bad_samples = self.bad_samples.saturating_add(1);
        self.good_samples = 0;
        if self.bad_samples >= MAG_BAD_SAMPLES_TO_DISTURBED {
            self.status = MagStatus::Disturbed;
        }
    }

    pub(super) const fn status(&self) -> MagStatus {
        self.status
    }

    pub(super) const fn field_ut(&self) -> f32 {
        self.field_ut
    }

    pub(super) const fn vector_ut(&self) -> Option<[f32; 3]> {
        self.vector_ut
    }

    pub(super) fn calibration_percent(&self) -> u8 {
        self.calibration.progress_percent()
    }
}
