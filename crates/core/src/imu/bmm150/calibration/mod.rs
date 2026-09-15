//! Run-time hard-iron and soft-iron calibration of the BMM150 magnetic
//! field.
//!
//! The magnetometer sits inside an enclosure that distorts the field:
//!
//! - Hard iron: magnetized parts near the sensor add a constant offset to
//!   every reading.
//! - Soft iron: metal near the sensor stretches and bends the field
//!   differently in each direction.
//!
//! Without distortion, the readings of all directions lie on a sphere
//! around zero. With distortion, they lie on an ellipsoid with its center
//! away from zero. The calibration fits that ellipsoid and maps it back onto
//! a sphere.
//!
//! How it works:
//!
//! 1. Warm-up: the first samples give the extrema (the smallest and largest
//!    value on each axis). Their midpoint is the first guess of the center,
//!    the fit origin.
//! 2. Coverage: while the user turns the board, samples from many
//!    directions go into the fit. One such round of collecting samples is a
//!    fit epoch.
//! 3. Fit: with enough samples and directions, an ellipsoid is fitted. A
//!    plausible fit becomes a candidate. A rejected fit starts a new epoch.
//! 4. Validation: the candidate is checked on new samples. It is accepted,
//!    or it is rejected and a new epoch starts.
//!
//! An epoch that takes too long without a fit attempt starts everything
//! over, extrema included (see `STALLED_EPOCH_SAMPLES`).

mod fit;
mod math;

use log::info;

use crate::imu::vec3;

use fit::{Candidate, MIN_FIT_SAMPLES, Model, NormalEquations, Validation};

/// Smallest raw field strength, in uT, that the calibration learns from.
///
/// Before the hard-iron offset is removed, the fields inside the enclosure
/// can be much stronger than the Earth's field. So the learning window is
/// wide. It only rejects samples that are near zero or look like an
/// overflow.
const LEARNING_FIELD_MIN_UT: f32 = 5.0;
/// Largest raw field strength, in uT, that the calibration learns from; see
/// `LEARNING_FIELD_MIN_UT`.
const LEARNING_FIELD_MAX_UT: f32 = 4000.0;
/// Smallest corrected field strength, in uT, that counts as the Earth's
/// field. Corrected fields have a strength of about 50 uT. The window leaves
/// room for noise and short disturbances.
const EARTH_FIELD_MIN_UT: f32 = 25.0;
/// Largest corrected field strength, in uT, that counts as the Earth's
/// field; see `EARTH_FIELD_MIN_UT`.
const EARTH_FIELD_MAX_UT: f32 = 80.0;

/// Smallest extent, in uT, the samples must span on every axis before a
/// fit is attempted.
const TARGET_SPAN_UT: f32 = 35.0;
/// Learnable samples needed before the fit origin is chosen.
const ORIGIN_WARMUP_SAMPLES: u32 = 36;
/// Extent, in uT, the samples must span on every axis before the fit origin
/// is chosen. Then the midpoint of the extrema is a usable guess of the
/// hard-iron offset.
const ORIGIN_MIN_SPAN_UT: f32 = 20.0;

/// Direction bins around the origin: 6 cube faces times 4 quadrants (see
/// `fit::direction_bin`). The `u32` bit masks have one bit per bin.
const DIRECTION_BIN_COUNT: usize = 24;
/// Distinct direction bins that must hold samples before a fit is
/// attempted.
const MIN_DIRECTION_BINS: u32 = 12;
/// Cube faces that must hold samples before a fit is attempted: all six,
/// so each axis has been seen pointing both ways.
const MIN_DIRECTION_FACES: u32 = 6;
/// Largest number of samples per direction bin that go into the fit. More
/// samples in a full bin are skipped. So a board held in one direction does
/// not get more weight than the other directions.
const MAX_SAMPLES_PER_DIRECTION_BIN: u8 = 32;
/// Samples that must go into the fit since the last fit attempt (or since
/// the start of the epoch) before the next attempt.
const REFIT_INTERVAL_SAMPLES: u16 = 16;
/// Distinct direction bins that must receive fit samples since the last fit
/// attempt (or since the start of the epoch) before the next attempt.
const MIN_REFIT_DIRECTION_BINS: u32 = 4;
/// Learnable samples one epoch may take without a fit attempt. After that,
/// the calibration starts over from nothing, extrema included. This is two
/// minutes at the magnetometer's 30 Hz. Samples during warm-up and during
/// validation do not count.
///
/// Why: a single disturbed sample (for example from a magnet that passes by)
/// stretches the extrema, and the extrema are never reset otherwise. Their
/// midpoint is then far from the real center. It puts all later samples
/// into the same few direction bins, so the coverage can never be complete.
const STALLED_EPOCH_SAMPLES: u32 = 3600;

// Progress reporting: the three phases add up to 99 %. 100 % means that a
// model is accepted.
/// Share of the progress for choosing the fit origin.
const ORIGIN_PHASE_PERCENT: f32 = 20.0;
/// Share of the progress for covering enough directions.
const COVERAGE_PHASE_PERCENT: f32 = 65.0;
/// Share of the progress for validating a candidate on fresh samples.
const VALIDATION_PHASE_PERCENT: f32 = 14.0;

/// Hard- and soft-iron calibration of the magnetometer, learned at run time.
///
/// Feed every raw field to [`Calibration::observe`] while the board is moved
/// in all directions. Once enough directions are covered, an ellipsoid is
/// fitted and checked against fresh samples; when it passes,
/// [`Calibration::is_ready`] turns true and [`Calibration::apply`] corrects
/// fields. [`Calibration::progress_percent`] tells the user how far along it
/// is.
pub struct Calibration {
    /// Least-squares sums of the samples in the current fit epoch. Reset when
    /// a new epoch starts.
    equations: NormalEquations,
    /// Smallest learnable raw field seen on each axis, in uT. Kept for the
    /// next fit epoch, but reset when an epoch stalls.
    min: [f32; 3],
    /// Largest learnable raw field seen on each axis, in uT. Kept for the
    /// next fit epoch, but reset when an epoch stalls.
    max: [f32; 3],
    /// Learnable samples seen since the start, including those not added to
    /// the fit. Reset when an epoch stalls. Stops at `u32::MAX`.
    samples: u32,
    /// The point that fit samples are measured from, in uT. `None` during
    /// warm-up. Set to the midpoint of the extrema after warm-up and at the
    /// start of every new epoch.
    fit_origin_ut: Option<[f32; 3]>,
    /// Samples added to `equations` in this epoch.
    fit_samples: u32,
    /// Learnable samples seen in this epoch while no candidate was being
    /// validated. Used to detect an epoch that cannot complete (see
    /// `STALLED_EPOCH_SAMPLES`).
    epoch_samples: u32,
    /// Samples added to `equations` since the last fit attempt.
    samples_since_fit: u16,
    /// Bit mask of the direction bins that received fit samples since the
    /// last fit attempt. Bit `n` is bin `n`.
    refit_direction_bins: u32,
    /// Bit mask of the direction bins seen in this epoch, also those that
    /// were already full. Bit `n` is bin `n`.
    direction_bins: u32,
    /// Bit mask of the six cube faces seen in this epoch. Bit `n` is face `n`.
    direction_faces: u8,
    /// Samples added to the fit from each direction bin, capped at
    /// `MAX_SAMPLES_PER_DIRECTION_BIN`.
    direction_bin_samples: [u8; DIRECTION_BIN_COUNT],
    /// A fitted model that is being validated. No new fit is attempted while
    /// it is set.
    candidate: Option<Candidate>,
    /// The accepted model. Once it is set, learning stops permanently.
    model: Option<Model>,
}

impl Default for Calibration {
    fn default() -> Self {
        Self::new()
    }
}

impl Calibration {
    /// No samples, no model.
    pub const fn new() -> Self {
        Self {
            equations: NormalEquations::new(),
            min: [f32::MAX; 3],
            max: [f32::MIN; 3],
            samples: 0,
            fit_origin_ut: None,
            fit_samples: 0,
            epoch_samples: 0,
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

    /// Whether a validated model is in use.
    pub fn is_ready(&self) -> bool {
        self.model.is_some()
    }

    /// Correct a raw body-frame field; unchanged before the model is ready.
    pub fn apply(&self, field_ut: [f32; 3]) -> [f32; 3] {
        self.model.map_or(field_ut, |model| model.apply(field_ut))
    }

    /// Learn from one raw body-frame field, in uT.
    ///
    /// Does nothing when a model is already accepted or when the field is not
    /// learnable.
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

        if self.candidate.is_none() {
            self.epoch_samples = self.epoch_samples.saturating_add(1);
            if self.epoch_samples > STALLED_EPOCH_SAMPLES {
                info!("BMM150 calibration stalled; starting over with fresh extrema");
                *self = Self::new();
                return;
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

    /// Fit a model to this epoch's samples. A plausible fit becomes the
    /// candidate. Otherwise a new epoch starts.
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

    /// Whether this epoch has enough samples, span, direction bins and faces
    /// for a fit attempt.
    fn has_minimum_coverage(&self) -> bool {
        self.fit_origin_ut.is_some()
            && self.fit_samples >= MIN_FIT_SAMPLES
            && self.minimum_span() >= TARGET_SPAN_UT
            && self.direction_bins.count_ones() >= MIN_DIRECTION_BINS
            && self.direction_faces.count_ones() >= MIN_DIRECTION_FACES
    }

    /// Midpoint of the observed extrema, in uT: a rough estimate of the
    /// hard-iron offset.
    fn coverage_origin(&self) -> [f32; 3] {
        vec3::scale(vec3::add(self.min, self.max), 0.5)
    }

    /// Smallest extent of the observed samples along any axis, in uT. 0
    /// before the first sample.
    fn minimum_span(&self) -> f32 {
        if self.samples == 0 {
            return 0.0;
        }
        vec3::sub(self.max, self.min)
            .into_iter()
            .fold(f32::MAX, f32::min)
    }

    /// Start a new fit epoch, after a fit or a candidate was rejected.
    ///
    /// A rejection means that the collected samples do not give a physically
    /// possible ellipsoid. More samples added to the same sums could make the
    /// balance between directions even worse. So this keeps the raw extrema,
    /// moves the fit origin to their midpoint and collects a new set of
    /// samples for the next fit.
    fn restart_fit_epoch(&mut self) {
        self.equations = NormalEquations::new();
        self.fit_origin_ut = Some(self.coverage_origin());
        self.fit_samples = 0;
        self.epoch_samples = 0;
        self.samples_since_fit = 0;
        self.refit_direction_bins = 0;
        self.direction_bins = 0;
        self.direction_faces = 0;
        self.direction_bin_samples = [0; DIRECTION_BIN_COUNT];
        self.candidate = None;
    }
}

/// A percentage as `u8`: limited to 0 to 100 and rounded down.
fn percent(value: f32) -> u8 {
    value.clamp(0.0, 100.0) as u8
}
