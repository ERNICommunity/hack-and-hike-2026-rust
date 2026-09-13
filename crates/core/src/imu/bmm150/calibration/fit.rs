//! Ellipsoid fitting and candidate validation for BMM150 calibration.

use super::math::{PARAMS, solve_linear, symmetric_eigen_3};
use crate::imu::vec3;

/// Balanced samples needed before a fit is attempted: more than ten
/// observations per fitted quadratic parameter.
pub const MIN_FIT_SAMPLES: u32 = 96;
/// A fit may run with slightly fewer weighted samples than the nominal minimum.
const MIN_FIT_WEIGHT_FRACTION: f32 = 0.75;

// Candidate validation is independent and direction-aware. At 30 Hz the
// maximum gives the user about ten seconds to cover six directions.
const VALIDATION_MIN_SAMPLES: u16 = 24;
const VALIDATION_MIN_BINS: u32 = 6;
const VALIDATION_MAX_SAMPLES: u16 = 300;
const MAX_VALIDATION_RMS_RELATIVE_ERROR: f32 = 0.15;
const MAX_VALIDATION_SINGLE_RELATIVE_ERROR: f32 = 0.35;
const MAX_VALIDATION_BAD_SAMPLES: u8 = 6;

/// Corrected fields are normalized to this magnitude.
pub const CALIBRATED_FIELD_RADIUS_UT: f32 = 50.0;
/// The fit works on fields divided by this scale so that quadratic and linear
/// terms stay comparable in f32.
const FIT_INPUT_SCALE_UT: f32 = 256.0;
const QUADRIC_SCALE_EPSILON: f32 = 1.0e-6;
const SHAPE_EIGEN_EPSILON: f32 = 1.0e-6;
const MAX_SHAPE_EIGEN_RATIO: f32 = 400.0;
const MAX_ELLIPSOID_RADIUS_UT: f32 = 300.0;
const MIN_ELLIPSOID_RADIUS_UT: f32 = 10.0;
const MAX_ALGEBRAIC_RMS: f32 = 0.15;
/// The fitted center must lie inside the observed extrema plus this margin.
const CENTER_MARGIN_FRACTION: f32 = 0.10;
const CENTER_MARGIN_UT: f32 = 5.0;

/// Hard-iron offset and soft-iron correction matrix.
#[derive(Clone, Copy)]
pub struct Model {
    pub center_ut: [f32; 3],
    correction: [[f32; 3]; 3],
}

impl Model {
    pub fn apply(&self, field_ut: [f32; 3]) -> [f32; 3] {
        vec3::matrix_vector(self.correction, vec3::sub(field_ut, self.center_ut))
    }
}

/// A fitted model being checked against fresh samples before it is trusted.
#[derive(Clone, Copy)]
pub struct Candidate {
    model: Model,
    samples: u16,
    direction_bins: u32,
    relative_error_squared_sum: f32,
    bad_samples: u8,
}

impl Candidate {
    pub const fn new(model: Model) -> Self {
        Self {
            model,
            samples: 0,
            direction_bins: 0,
            relative_error_squared_sum: 0.0,
            bad_samples: 0,
        }
    }

    /// Fraction of the validation requirement met so far.
    pub fn progress(self) -> f32 {
        let samples = f32::from(self.samples) / f32::from(VALIDATION_MIN_SAMPLES);
        let bins = self.direction_bins.count_ones() as f32 / VALIDATION_MIN_BINS as f32;
        samples.min(bins).clamp(0.0, 1.0)
    }
}

pub enum Validation {
    Pending(Candidate),
    Accepted(Model),
    Rejected,
}

pub fn validate_candidate(mut candidate: Candidate, field_ut: [f32; 3]) -> Validation {
    // Diversity is measured around the fitted center, the best physical
    // reference available.
    let bin = direction_bin(field_ut, candidate.model.center_ut);
    candidate.samples = candidate.samples.saturating_add(1);
    candidate.direction_bins |= 1 << bin;

    let corrected_strength = vec3::norm(candidate.model.apply(field_ut));
    let relative_error =
        (corrected_strength - CALIBRATED_FIELD_RADIUS_UT).abs() / CALIBRATED_FIELD_RADIUS_UT;
    candidate.relative_error_squared_sum += relative_error * relative_error;
    if !relative_error.is_finite() || relative_error > MAX_VALIDATION_SINGLE_RELATIVE_ERROR {
        candidate.bad_samples = candidate.bad_samples.saturating_add(1);
    }

    let enough_samples = candidate.samples >= VALIDATION_MIN_SAMPLES;
    let enough_directions = candidate.direction_bins.count_ones() >= VALIDATION_MIN_BINS;
    if enough_samples && enough_directions {
        let rms =
            libm::sqrtf(candidate.relative_error_squared_sum / f32::from(candidate.samples.max(1)));
        return if rms.is_finite()
            && rms <= MAX_VALIDATION_RMS_RELATIVE_ERROR
            && candidate.bad_samples <= MAX_VALIDATION_BAD_SAMPLES
        {
            Validation::Accepted(candidate.model)
        } else {
            Validation::Rejected
        };
    }

    if candidate.samples >= VALIDATION_MAX_SAMPLES
        || candidate.bad_samples > MAX_VALIDATION_BAD_SAMPLES
    {
        return Validation::Rejected;
    }
    Validation::Pending(candidate)
}

/// One of 24 direction bins (6 cube faces times 4 quadrants) of a field
/// relative to `origin_ut`; used to judge how well the sphere is covered.
pub fn direction_bin(field_ut: [f32; 3], origin_ut: [f32; 3]) -> usize {
    let [x, y, z] = vec3::sub(field_ut, origin_ut);
    let (face, first, second) = if x.abs() >= y.abs() && x.abs() >= z.abs() {
        (if x >= 0.0 { 0 } else { 1 }, y, z)
    } else if y.abs() >= z.abs() {
        (if y >= 0.0 { 2 } else { 3 }, x, z)
    } else {
        (if z >= 0.0 { 4 } else { 5 }, x, y)
    };
    let quadrant = usize::from(first >= 0.0) << 1 | usize::from(second >= 0.0);
    face * 4 + quadrant
}

/// Least-squares normal equations of the algebraic ellipsoid fit.
pub struct NormalEquations {
    matrix: [[f32; PARAMS]; PARAMS],
    rhs: [f32; PARAMS],
    weight_sum: f32,
}

impl NormalEquations {
    pub const fn new() -> Self {
        Self {
            matrix: [[0.0; PARAMS]; PARAMS],
            rhs: [0.0; PARAMS],
            weight_sum: 0.0,
        }
    }

    pub fn accumulate(&mut self, field_ut: [f32; 3], origin_ut: [f32; 3]) {
        let [x, y, z] = vec3::scale(vec3::sub(field_ut, origin_ut), 1.0 / FIT_INPUT_SCALE_UT);
        let feature = [
            x * x,
            y * y,
            z * z,
            2.0 * x * y,
            2.0 * x * z,
            2.0 * y * z,
            x,
            y,
            z,
        ];

        for (row, (matrix_row, rhs)) in self.matrix.iter_mut().zip(&mut self.rhs).enumerate() {
            *rhs += feature[row];
            for (cell, value) in matrix_row.iter_mut().zip(feature) {
                *cell += feature[row] * value;
            }
        }
        self.weight_sum += 1.0;
    }

    /// Root-mean-square algebraic residual of `parameters`.
    fn rms(&self, parameters: [f32; PARAMS]) -> f32 {
        let mut quadratic = 0.0;
        let mut linear = 0.0;
        for (row, (matrix_row, rhs)) in self.matrix.iter().zip(&self.rhs).enumerate() {
            linear += parameters[row] * rhs;
            for (cell, parameter) in matrix_row.iter().zip(parameters) {
                quadratic += parameters[row] * cell * parameter;
            }
        }
        let sse = (quadratic - 2.0 * linear + self.weight_sum).max(0.0);
        libm::sqrtf(sse / self.weight_sum)
    }
}

/// Fit an ellipsoid to the accumulated samples. `origin_ut` is the point the
/// samples were accumulated around; `min`/`max` are the observed extrema used
/// to sanity-check the fitted center.
pub fn fit_model(
    equations: &NormalEquations,
    origin_ut: [f32; 3],
    min: [f32; 3],
    max: [f32; 3],
) -> Option<Model> {
    if equations.weight_sum < MIN_FIT_SAMPLES as f32 * MIN_FIT_WEIGHT_FRACTION {
        return None;
    }

    let inverse_weight = 1.0 / equations.weight_sum;
    let scaled_matrix = equations
        .matrix
        .map(|row| row.map(|value| value * inverse_weight));
    let scaled_rhs = equations.rhs.map(|value| value * inverse_weight);
    let parameters = solve_linear(scaled_matrix, scaled_rhs)?;

    let rms = equations.rms(parameters);
    if !rms.is_finite() || rms > MAX_ALGEBRAIC_RMS {
        return None;
    }

    let q = [
        [parameters[0], parameters[3], parameters[4]],
        [parameters[3], parameters[1], parameters[5]],
        [parameters[4], parameters[5], parameters[2]],
    ];
    let linear = [parameters[6], parameters[7], parameters[8]];
    let center_normalized = vec3::scale(solve_linear(q, linear)?, -0.5);

    let scale = 1.0 + vec3::dot(center_normalized, vec3::matrix_vector(q, center_normalized));
    if !scale.is_finite() || scale.abs() < QUADRIC_SCALE_EPSILON {
        return None;
    }
    let shape = q.map(|row| row.map(|value| value / scale));

    let eigen = symmetric_eigen_3(shape)?;
    let mut sqrt_eigen = [0.0; 3];
    for (sqrt_value, eigenvalue) in sqrt_eigen.iter_mut().zip(eigen.values) {
        if !eigenvalue.is_finite() || eigenvalue <= SHAPE_EIGEN_EPSILON {
            return None;
        }
        *sqrt_value = libm::sqrtf(eigenvalue);
        let radius_ut = FIT_INPUT_SCALE_UT / *sqrt_value;
        if !(MIN_ELLIPSOID_RADIUS_UT..=MAX_ELLIPSOID_RADIUS_UT).contains(&radius_ut) {
            return None;
        }
    }
    let min_eigen = eigen.values.iter().copied().fold(f32::MAX, f32::min);
    let max_eigen = eigen.values.iter().copied().fold(0.0f32, f32::max);
    if max_eigen / min_eigen > MAX_SHAPE_EIGEN_RATIO {
        return None;
    }

    let center_ut = vec3::add(
        origin_ut,
        vec3::scale(center_normalized, FIT_INPUT_SCALE_UT),
    );
    if !center_is_covered(center_ut, min, max) {
        return None;
    }

    // Symmetric square root V * sqrt(D) * V^T, scaled to the target radius and
    // undoing the input normalization: a direct raw-uT to uT correction
    // matrix without an arbitrary rotation.
    let output_scale = CALIBRATED_FIELD_RADIUS_UT / FIT_INPUT_SCALE_UT;
    let mut correction = [[0.0; 3]; 3];
    for (row, correction_row) in correction.iter_mut().enumerate() {
        for (col, cell) in correction_row.iter_mut().enumerate() {
            let value: f32 = (0..3)
                .map(|axis| eigen.vectors[row][axis] * sqrt_eigen[axis] * eigen.vectors[col][axis])
                .sum();
            *cell = value * output_scale;
            if !cell.is_finite() {
                return None;
            }
        }
    }

    Some(Model {
        center_ut,
        correction,
    })
}

fn center_is_covered(center_ut: [f32; 3], min: [f32; 3], max: [f32; 3]) -> bool {
    (0..3).all(|axis| {
        let margin = CENTER_MARGIN_FRACTION * (max[axis] - min[axis]) + CENTER_MARGIN_UT;
        (min[axis] - margin..=max[axis] + margin).contains(&center_ut[axis])
    })
}
