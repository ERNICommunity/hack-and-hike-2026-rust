//! Runtime hard/soft-iron calibration for the BMM150 magnetic field.
//!
//! Calibration lifecycle and acceptance remain here. Ellipsoid fitting and its
//! fixed-size numerical helpers stay feature-local in private child modules.

mod fit;
mod math;

use fit::{CALIBRATION_MIN_FIT_SAMPLES, Candidate, CandidateValidation, Model};
use math::{PARAMS, bit_count_u8, bit_count_u32, clamp_f32, dot3, min_f32, sqrt_approx};

/// Compensated BMM150 fields inside the CoreS3 enclosure can be far larger than
/// the Earth's field before hard-iron removal. Keep the learning window broad,
/// but reject near-zero/overflow-like samples.
const LEARNING_FIELD_MIN_UT: f32 = 5.0;
const LEARNING_FIELD_MAX_UT: f32 = 4000.0;

/// Post-calibration magnitude gate. The calibration intentionally normalizes
/// the accepted ellipsoid to 50 uT, so a much wider 5..150 uT window hid bad
/// fits. This still leaves generous room for noise and transient disturbances.
pub(crate) const GOOD_FIELD_MIN_UT: f32 = 25.0;
pub(crate) const GOOD_FIELD_MAX_UT: f32 = 80.0;

const CALIBRATION_TARGET_SPAN_UT: f32 = 35.0;
// The evolving-extrema coverage origin corrects an imperfect early midpoint, so
// a full two seconds of warm-up is unnecessary. The independent span condition
// still prevents choosing an origin before the device has moved in 3-D.
const ORIGIN_WARMUP_SAMPLES: u32 = 36;
const ORIGIN_MIN_SPAN_UT: f32 = 20.0;

// Divide the sphere into six dominant-axis faces, each split into four
// quadrants. Twelve well-distributed sectors plus all six +/-axis faces are
// enough to condition a nine-parameter ellipsoid. A fresh-sample validation
// phase still has to accept every provisional fit before it can be used for
// heading.
const DIRECTION_BIN_COUNT: usize = 24;
const MIN_DIRECTION_BINS: u32 = 12;
const MIN_DIRECTION_FACES: u32 = 6;
const MAX_SAMPLES_PER_DIRECTION_BIN: u8 = 32;
const REFIT_INTERVAL_SAMPLES: u16 = 16;
const MIN_REFIT_DIRECTION_BINS: u32 = 4;

/// Allocation-free online full-ellipsoid magnetometer calibration.
///
/// The fitter accumulates 9x9 normal equations for the general quadratic terms
/// `x^2, y^2, z^2, 2xy, 2xz, 2yz, x, y, z`, but only up to a bounded number of
/// samples per 3-D direction sector. A candidate model is then checked against
/// fresh, directionally diverse samples before it is frozen.
pub(crate) struct Calibration {
    normal: [[f32; PARAMS]; PARAMS],
    rhs: [f32; PARAMS],
    min: [f32; 3],
    max: [f32; 3],
    samples: u32,
    fit_origin_ut: Option<[f32; 3]>,
    fit_samples: u32,
    weight_sum: f32,
    samples_since_fit: u16,
    refit_direction_bins: u32,
    direction_bins: u32,
    direction_faces: u8,
    direction_bin_samples: [u8; DIRECTION_BIN_COUNT],
    candidate: Option<Candidate>,
    model: Option<Model>,
}

impl Calibration {
    pub(crate) const fn new() -> Self {
        Self {
            normal: [[0.0; PARAMS]; PARAMS],
            rhs: [0.0; PARAMS],
            min: [f32::MAX; 3],
            max: [f32::MIN; 3],
            samples: 0,
            fit_origin_ut: None,
            fit_samples: 0,
            weight_sum: 0.0,
            samples_since_fit: 0,
            refit_direction_bins: 0,
            direction_bins: 0,
            direction_faces: 0,
            direction_bin_samples: [0; DIRECTION_BIN_COUNT],
            candidate: None,
            model: None,
        }
    }

    /// Learn one Bosch-compensated body-frame vector while calibration is not
    /// ready. Calibration deliberately has three phases:
    ///
    /// 1. establish a numerically safe local origin,
    /// 2. collect a balanced set of samples across the 3-D sphere and fit,
    /// 3. validate the provisional correction on fresh measurements.
    ///
    /// Rejected fits never discard the complete learning history. Instead the
    /// calibrator opens a fresh balanced per-direction sampling epoch and waits
    /// for genuinely new, directionally diverse data before fitting again. Once
    /// a validated model is accepted it is frozen so external magnetic
    /// disturbances cannot be learned as enclosure hard/soft iron.
    pub(crate) fn observe(&mut self, field_ut: [f32; 3]) {
        if self.model.is_some() || !raw_sample_is_plausible(field_ut) {
            return;
        }

        for axis in 0..3 {
            self.min[axis] = math::min_f32(self.min[axis], field_ut[axis]);
            self.max[axis] = math::max_f32(self.max[axis], field_ut[axis]);
        }
        self.samples = self.samples.saturating_add(1);

        if self.fit_origin_ut.is_none() {
            if self.samples >= ORIGIN_WARMUP_SAMPLES && self.minimum_span() >= ORIGIN_MIN_SPAN_UT {
                self.fit_origin_ut = Some([
                    0.5 * (self.min[0] + self.max[0]),
                    0.5 * (self.min[1] + self.max[1]),
                    0.5 * (self.min[2] + self.max[2]),
                ]);
            }
            return;
        }

        // The fit origin stays fixed for numerical conditioning, but coverage
        // classification follows the evolving extrema midpoint. The early
        // warm-up midpoint can be biased if the user has not yet explored the
        // opposite side of the field sphere; freezing that midpoint for sector
        // classification was the main reason real devices could park near 70%.
        let direction_bin = fit::direction_bin(field_ut, self.coverage_origin());

        if let Some(candidate) = self.candidate.take() {
            match fit::validate_candidate(candidate, field_ut) {
                CandidateValidation::Pending(candidate) => self.candidate = Some(candidate),
                CandidateValidation::Accepted(model) => self.model = Some(model),
                CandidateValidation::Rejected => {}
            }
            if self.model.is_some() || self.fit_origin_ut.is_none() {
                return;
            }
        }

        self.direction_bins |= 1u32 << direction_bin;
        self.direction_faces |= 1u8 << (direction_bin / 4);

        if self.direction_bin_samples[direction_bin] < MAX_SAMPLES_PER_DIRECTION_BIN {
            self.direction_bin_samples[direction_bin] += 1;
            fit::accumulate(
                &mut self.normal,
                &mut self.rhs,
                &mut self.weight_sum,
                field_ut,
                self.fit_origin_ut.unwrap_or([0.0; 3]),
            );
            self.fit_samples = self.fit_samples.saturating_add(1);
            self.samples_since_fit = self.samples_since_fit.saturating_add(1);
            self.refit_direction_bins |= 1u32 << direction_bin;
        }

        if self.candidate.is_none()
            && self.has_minimum_coverage()
            && self.samples_since_fit >= REFIT_INTERVAL_SAMPLES
            && bit_count_u32(self.refit_direction_bins) >= MIN_REFIT_DIRECTION_BINS
        {
            self.samples_since_fit = 0;
            self.refit_direction_bins = 0;
            match fit::fit_model(
                &self.normal,
                &self.rhs,
                self.weight_sum,
                self.fit_origin_ut,
                self.min,
                self.max,
            ) {
                Some(model) => {
                    self.candidate = Some(Candidate::new(model));
                    self.open_balanced_refit_epoch();
                }
                None => self.open_balanced_refit_epoch(),
            }
        }
    }

    pub(crate) fn is_ready(&self) -> bool {
        self.model.is_some()
    }

    /// Report meaningful phase progress rather than sample-count optimism.
    ///
    /// 0..20: origin warm-up
    /// 20..85: balanced 3-D fitting coverage
    /// 85..99: fresh-sample candidate validation
    /// 100: validated and frozen
    pub(crate) fn progress_percent(&self) -> u8 {
        if self.model.is_some() {
            return 100;
        }
        if self.samples == 0 {
            return 0;
        }

        if self.fit_origin_ut.is_none() {
            let sample_progress =
                clamp_f32(self.samples as f32 / ORIGIN_WARMUP_SAMPLES as f32, 0.0, 1.0);
            let span_progress = clamp_f32(self.minimum_span() / ORIGIN_MIN_SPAN_UT, 0.0, 1.0);
            return (20.0 * min_f32(sample_progress, span_progress)) as u8;
        }

        let sample_progress = clamp_f32(
            self.fit_samples as f32 / CALIBRATION_MIN_FIT_SAMPLES as f32,
            0.0,
            1.0,
        );
        let span_progress = clamp_f32(self.minimum_span() / CALIBRATION_TARGET_SPAN_UT, 0.0, 1.0);
        let direction_progress = clamp_f32(
            bit_count_u32(self.direction_bins) as f32 / MIN_DIRECTION_BINS as f32,
            0.0,
            1.0,
        );
        let face_progress = clamp_f32(
            bit_count_u8(self.direction_faces) as f32 / MIN_DIRECTION_FACES as f32,
            0.0,
            1.0,
        );
        let coverage_progress = min_f32(
            min_f32(sample_progress, span_progress),
            min_f32(direction_progress, face_progress),
        );
        let coverage_percent = 20.0 + 65.0 * coverage_progress;

        if let Some(candidate) = self.candidate {
            return (85.0 + 14.0 * fit::validation_progress(candidate)) as u8;
        }

        (coverage_percent as u8).min(85)
    }

    pub(crate) fn apply(&self, field_ut: [f32; 3]) -> [f32; 3] {
        self.model
            .map(|model| model.apply(field_ut))
            .unwrap_or(field_ut)
    }

    fn has_minimum_coverage(&self) -> bool {
        self.fit_origin_ut.is_some()
            && self.fit_samples >= CALIBRATION_MIN_FIT_SAMPLES
            && self.minimum_span() >= CALIBRATION_TARGET_SPAN_UT
            && bit_count_u32(self.direction_bins) >= MIN_DIRECTION_BINS
            && bit_count_u8(self.direction_faces) >= MIN_DIRECTION_FACES
    }

    fn coverage_origin(&self) -> [f32; 3] {
        [
            0.5 * (self.min[0] + self.max[0]),
            0.5 * (self.min[1] + self.max[1]),
            0.5 * (self.min[2] + self.max[2]),
        ]
    }

    fn minimum_span(&self) -> f32 {
        if self.samples == 0 {
            return 0.0;
        }
        min_f32(
            self.max[0] - self.min[0],
            min_f32(self.max[1] - self.min[1], self.max[2] - self.min[2]),
        )
    }

    fn open_balanced_refit_epoch(&mut self) {
        // Keep all accumulated normal equations/extrema, but reopen each
        // direction's bounded quota so fresh measurements can continue to
        // improve a rejected or numerically invalid fit. This avoids the old
        // user-visible 85..99 -> 0 restart while preserving balanced sampling.
        self.direction_bin_samples = [0; DIRECTION_BIN_COUNT];
        self.samples_since_fit = 0;
        self.refit_direction_bins = 0;
    }
}

fn raw_sample_is_plausible(field_ut: [f32; 3]) -> bool {
    let strength = vector_length(field_ut);
    (LEARNING_FIELD_MIN_UT..=LEARNING_FIELD_MAX_UT).contains(&strength)
}

pub(crate) fn vector_length(value: [f32; 3]) -> f32 {
    sqrt_approx(dot3(value, value))
}
