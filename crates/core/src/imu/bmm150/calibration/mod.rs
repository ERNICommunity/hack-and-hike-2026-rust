//! Runtime hard/soft-iron calibration for the BMM150 magnetic field.
//!
//! The magnetometer sits inside an enclosure that distorts the field. While
//! the user moves the device, samples are collected until they cover the
//! sphere well enough to fit an ellipsoid; a fitted model is then validated on
//! fresh samples before it is accepted.

mod fit;
mod math;

use log::info;

use crate::imu::vec3;

use fit::{Candidate, MIN_FIT_SAMPLES, Model, NormalEquations, Validation};

/// Compensated fields inside the enclosure can be far larger than the Earth's
/// field before hard-iron removal. Keep the learning window broad, but reject
/// near-zero and overflow-like samples.
const LEARNING_FIELD_MIN_UT: f32 = 5.0;
const LEARNING_FIELD_MAX_UT: f32 = 4000.0;
/// Corrected fields are normalized to 50 uT; this window leaves room for noise
/// and transient disturbances.
const EARTH_FIELD_MIN_UT: f32 = 25.0;
const EARTH_FIELD_MAX_UT: f32 = 80.0;

const TARGET_SPAN_UT: f32 = 35.0;
const ORIGIN_WARMUP_SAMPLES: u32 = 36;
const ORIGIN_MIN_SPAN_UT: f32 = 20.0;

const DIRECTION_BIN_COUNT: usize = 24;
const MIN_DIRECTION_BINS: u32 = 12;
const MIN_DIRECTION_FACES: u32 = 6;
const MAX_SAMPLES_PER_DIRECTION_BIN: u8 = 32;
const REFIT_INTERVAL_SAMPLES: u16 = 16;
const MIN_REFIT_DIRECTION_BINS: u32 = 4;

// Progress reporting: the three phases add up to 99 %, 100 % means accepted.
const ORIGIN_PHASE_PERCENT: f32 = 20.0;
const COVERAGE_PHASE_PERCENT: f32 = 65.0;
const VALIDATION_PHASE_PERCENT: f32 = 14.0;

pub struct Calibration {
    equations: NormalEquations,
    min: [f32; 3],
    max: [f32; 3],
    samples: u32,
    fit_origin_ut: Option<[f32; 3]>,
    fit_samples: u32,
    samples_since_fit: u16,
    refit_direction_bins: u32,
    direction_bins: u32,
    direction_faces: u8,
    direction_bin_samples: [u8; DIRECTION_BIN_COUNT],
    candidate: Option<Candidate>,
    model: Option<Model>,
}

impl Default for Calibration {
    fn default() -> Self {
        Self::new()
    }
}

impl Calibration {
    pub const fn new() -> Self {
        Self {
            equations: NormalEquations::new(),
            min: [f32::MAX; 3],
            max: [f32::MIN; 3],
            samples: 0,
            fit_origin_ut: None,
            fit_samples: 0,
            samples_since_fit: 0,
            refit_direction_bins: 0,
            direction_bins: 0,
            direction_faces: 0,
            direction_bin_samples: [0; DIRECTION_BIN_COUNT],
            candidate: None,
            model: None,
        }
    }

    /// Whether a raw field could be a distorted Earth field worth learning from.
    pub fn is_learnable(field_ut: [f32; 3]) -> bool {
        (LEARNING_FIELD_MIN_UT..=LEARNING_FIELD_MAX_UT).contains(&vec3::norm(field_ut))
    }

    /// Whether a corrected field magnitude looks like the Earth's field.
    pub fn is_earth_field(strength_ut: f32) -> bool {
        (EARTH_FIELD_MIN_UT..=EARTH_FIELD_MAX_UT).contains(&strength_ut)
    }

    pub fn is_ready(&self) -> bool {
        self.model.is_some()
    }

    pub fn apply(&self, field_ut: [f32; 3]) -> [f32; 3] {
        self.model.map_or(field_ut, |model| model.apply(field_ut))
    }

    /// Learn from one raw body-frame field.
    pub fn observe(&mut self, field_ut: [f32; 3]) {
        if self.model.is_some() || !Self::is_learnable(field_ut) {
            return;
        }

        for ((min, max), value) in self.min.iter_mut().zip(&mut self.max).zip(field_ut) {
            *min = min.min(value);
            *max = max.max(value);
        }
        self.samples = self.samples.saturating_add(1);

        let Some(fit_origin) = self.fit_origin_ut else {
            if self.samples >= ORIGIN_WARMUP_SAMPLES && self.minimum_span() >= ORIGIN_MIN_SPAN_UT {
                self.fit_origin_ut = Some(self.coverage_origin());
            }
            return;
        };

        if let Some(candidate) = self.candidate.take() {
            match fit::validate_candidate(candidate, field_ut) {
                Validation::Pending(candidate) => self.candidate = Some(candidate),
                Validation::Accepted(model) => {
                    info!("BMM150 calibration accepted");
                    self.model = Some(model);
                    return;
                }
                Validation::Rejected => {
                    info!("BMM150 calibration candidate rejected; collecting a fresh sample set");
                    self.restart_fit_epoch();
                    return;
                }
            }
        }

        let bin = fit::direction_bin(field_ut, self.coverage_origin());
        self.direction_bins |= 1 << bin;
        self.direction_faces |= 1 << (bin / 4);
        if self.direction_bin_samples[bin] < MAX_SAMPLES_PER_DIRECTION_BIN {
            self.direction_bin_samples[bin] += 1;
            self.equations.accumulate(field_ut, fit_origin);
            self.fit_samples = self.fit_samples.saturating_add(1);
            self.samples_since_fit = self.samples_since_fit.saturating_add(1);
            self.refit_direction_bins |= 1 << bin;
        }

        if self.candidate.is_none()
            && self.has_minimum_coverage()
            && self.samples_since_fit >= REFIT_INTERVAL_SAMPLES
            && self.refit_direction_bins.count_ones() >= MIN_REFIT_DIRECTION_BINS
        {
            self.samples_since_fit = 0;
            self.refit_direction_bins = 0;
            self.try_fit(fit_origin);
        }
    }

    fn try_fit(&mut self, fit_origin: [f32; 3]) {
        match fit::fit_model(&self.equations, fit_origin, self.min, self.max) {
            Some(model) => {
                info!(
                    "BMM150 calibration candidate: samples={} bins={} faces={} span={}uT",
                    self.fit_samples,
                    self.direction_bins.count_ones(),
                    self.direction_faces.count_ones(),
                    self.minimum_span()
                );
                self.candidate = Some(Candidate::new(model));
            }
            None => {
                info!(
                    "BMM150 calibration fit rejected: samples={} bins={} faces={} span={}uT; collecting a fresh sample set",
                    self.fit_samples,
                    self.direction_bins.count_ones(),
                    self.direction_faces.count_ones(),
                    self.minimum_span()
                );
                self.restart_fit_epoch();
            }
        }
    }

    /// 0 to 100 %, where 100 means a validated model is in use.
    pub fn progress_percent(&self) -> u8 {
        if self.model.is_some() {
            return 100;
        }
        if self.samples == 0 {
            return 0;
        }
        if let Some(candidate) = self.candidate {
            return percent(
                ORIGIN_PHASE_PERCENT
                    + COVERAGE_PHASE_PERCENT
                    + VALIDATION_PHASE_PERCENT * candidate.progress(),
            );
        }
        if self.fit_origin_ut.is_none() {
            let samples = self.samples as f32 / ORIGIN_WARMUP_SAMPLES as f32;
            let span = self.minimum_span() / ORIGIN_MIN_SPAN_UT;
            return percent(ORIGIN_PHASE_PERCENT * samples.min(span).clamp(0.0, 1.0));
        }

        let samples = self.fit_samples as f32 / MIN_FIT_SAMPLES as f32;
        let span = self.minimum_span() / TARGET_SPAN_UT;
        let bins = self.direction_bins.count_ones() as f32 / MIN_DIRECTION_BINS as f32;
        let faces = self.direction_faces.count_ones() as f32 / MIN_DIRECTION_FACES as f32;
        let coverage = samples.min(span).min(bins).min(faces).clamp(0.0, 1.0);
        percent(ORIGIN_PHASE_PERCENT + COVERAGE_PHASE_PERCENT * coverage)
    }

    fn has_minimum_coverage(&self) -> bool {
        self.fit_origin_ut.is_some()
            && self.fit_samples >= MIN_FIT_SAMPLES
            && self.minimum_span() >= TARGET_SPAN_UT
            && self.direction_bins.count_ones() >= MIN_DIRECTION_BINS
            && self.direction_faces.count_ones() >= MIN_DIRECTION_FACES
    }

    /// Midpoint of the observed extrema, a rough hard-iron estimate.
    fn coverage_origin(&self) -> [f32; 3] {
        vec3::scale(vec3::add(self.min, self.max), 0.5)
    }

    /// Smallest extent of the observed samples along any axis.
    fn minimum_span(&self) -> f32 {
        if self.samples == 0 {
            return 0.0;
        }
        vec3::sub(self.max, self.min)
            .into_iter()
            .fold(f32::MAX, f32::min)
    }

    /// A rejected fit says the accumulated equations do not describe a
    /// physically acceptable ellipsoid. Adding a few more directions to that
    /// history could unbalance it further, so keep the raw extrema, recenter
    /// on their midpoint and build the next fit from a fresh sample set.
    fn restart_fit_epoch(&mut self) {
        self.equations = NormalEquations::new();
        self.fit_origin_ut = Some(self.coverage_origin());
        self.fit_samples = 0;
        self.samples_since_fit = 0;
        self.refit_direction_bins = 0;
        self.direction_bins = 0;
        self.direction_faces = 0;
        self.direction_bin_samples = [0; DIRECTION_BIN_COUNT];
        self.candidate = None;
    }
}

/// Truncate a 0..=100 float percentage to `u8`.
fn percent(value: f32) -> u8 {
    value.clamp(0.0, 100.0) as u8
}
