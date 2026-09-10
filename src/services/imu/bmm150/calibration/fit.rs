//! Ellipsoid fitting and provisional-model validation for BMM150 calibration.

use super::math::{
    PARAMS, abs_f32, bit_count_u32, clamp_f32, dot3, matrix_vector, max_f32, min_f32,
    solve_linear_3, solve_linear_9, sqrt_approx, symmetric_eigen_3,
};

// Ninety-six balanced samples still provide more than ten observations per
// fitted quadratic parameter while the independent span/sector/face gates keep
// the geometry well conditioned. The previous 144-sample floor made calibration
// noticeably slow on a 30 Hz magnetometer without adding a separate quality test.
pub(super) const CALIBRATION_MIN_FIT_SAMPLES: u32 = 96;

// Candidate validation remains independent and direction-aware, but it does not
// need another 48-sample collection after an already balanced full-ellipsoid fit.
const CANDIDATE_VALIDATION_MIN_SAMPLES: u16 = 24;
const CANDIDATE_VALIDATION_MIN_BINS: u32 = 6;
const CANDIDATE_VALIDATION_MAX_SAMPLES: u16 = 72;
const MAX_CANDIDATE_RMS_RELATIVE_ERROR: f32 = 0.15;
const MAX_CANDIDATE_SINGLE_RELATIVE_ERROR: f32 = 0.35;
const MAX_CANDIDATE_BAD_SAMPLES: u8 = 6;

const CALIBRATED_FIELD_RADIUS_UT: f32 = 50.0;
const FIT_INPUT_SCALE_UT: f32 = 1024.0;
const QUADRIC_SCALE_EPSILON: f32 = 1.0e-6;
const SHAPE_EIGEN_EPSILON: f32 = 1.0e-6;
const MAX_SHAPE_EIGEN_RATIO: f32 = 400.0;
const MAX_ELLIPSOID_RADIUS_UT: f32 = 300.0;
const MIN_ELLIPSOID_RADIUS_UT: f32 = 10.0;
const MAX_ALGEBRAIC_RMS: f32 = 0.15;

#[derive(Clone, Copy)]
pub(super) struct Model {
    pub(super) center_ut: [f32; 3],
    correction: [[f32; 3]; 3],
}

impl Model {
    pub(super) fn apply(&self, field_ut: [f32; 3]) -> [f32; 3] {
        let centered = [
            field_ut[0] - self.center_ut[0],
            field_ut[1] - self.center_ut[1],
            field_ut[2] - self.center_ut[2],
        ];
        matrix_vector(self.correction, centered)
    }
}

#[derive(Clone, Copy)]
pub(super) struct Candidate {
    model: Model,
    samples: u16,
    direction_bins: u32,
    relative_error_squared_sum: f32,
    bad_samples: u8,
}

impl Candidate {
    pub(super) const fn new(model: Model) -> Self {
        Self {
            model,
            samples: 0,
            direction_bins: 0,
            relative_error_squared_sum: 0.0,
            bad_samples: 0,
        }
    }
}

pub(super) enum CandidateValidation {
    Pending(Candidate),
    Accepted(Model),
    Rejected,
}

pub(super) fn validate_candidate(
    mut candidate: Candidate,
    field_ut: [f32; 3],
) -> CandidateValidation {
    // Validation diversity is measured around the fitted hard-iron center,
    // which is a better physical reference than either the warm-up or the
    // evolving extrema midpoint.
    let direction_bin = direction_bin(field_ut, candidate.model.center_ut);
    candidate.samples = candidate.samples.saturating_add(1);
    candidate.direction_bins |= 1u32 << direction_bin;

    let corrected_strength = vector_length(candidate.model.apply(field_ut));
    let relative_error =
        abs_f32(corrected_strength - CALIBRATED_FIELD_RADIUS_UT) / CALIBRATED_FIELD_RADIUS_UT;
    candidate.relative_error_squared_sum += relative_error * relative_error;
    if !relative_error.is_finite() || relative_error > MAX_CANDIDATE_SINGLE_RELATIVE_ERROR {
        candidate.bad_samples = candidate.bad_samples.saturating_add(1);
    }

    let enough_samples = candidate.samples >= CANDIDATE_VALIDATION_MIN_SAMPLES;
    let enough_directions =
        bit_count_u32(candidate.direction_bins) >= CANDIDATE_VALIDATION_MIN_BINS;

    if enough_samples && enough_directions {
        let rms =
            sqrt_approx(candidate.relative_error_squared_sum / candidate.samples.max(1) as f32);
        if rms.is_finite()
            && rms <= MAX_CANDIDATE_RMS_RELATIVE_ERROR
            && candidate.bad_samples <= MAX_CANDIDATE_BAD_SAMPLES
        {
            return CandidateValidation::Accepted(candidate.model);
        }

        return CandidateValidation::Rejected;
    }

    if candidate.samples >= CANDIDATE_VALIDATION_MAX_SAMPLES
        || candidate.bad_samples > MAX_CANDIDATE_BAD_SAMPLES
    {
        return CandidateValidation::Rejected;
    }

    CandidateValidation::Pending(candidate)
}

pub(super) fn validation_progress(candidate: Candidate) -> f32 {
    let validation_samples = clamp_f32(
        candidate.samples as f32 / CANDIDATE_VALIDATION_MIN_SAMPLES as f32,
        0.0,
        1.0,
    );
    let validation_bins = clamp_f32(
        bit_count_u32(candidate.direction_bins) as f32 / CANDIDATE_VALIDATION_MIN_BINS as f32,
        0.0,
        1.0,
    );
    min_f32(validation_samples, validation_bins)
}

pub(super) fn direction_bin(field_ut: [f32; 3], origin_ut: [f32; 3]) -> usize {
    let x = field_ut[0] - origin_ut[0];
    let y = field_ut[1] - origin_ut[1];
    let z = field_ut[2] - origin_ut[2];
    let ax = abs_f32(x);
    let ay = abs_f32(y);
    let az = abs_f32(z);

    let (face, first, second) = if ax >= ay && ax >= az {
        (if x >= 0.0 { 0 } else { 1 }, y, z)
    } else if ay >= az {
        (if y >= 0.0 { 2 } else { 3 }, x, z)
    } else {
        (if z >= 0.0 { 4 } else { 5 }, x, y)
    };
    let quadrant = (if first >= 0.0 { 2 } else { 0 }) | (if second >= 0.0 { 1 } else { 0 });
    face * 4 + quadrant
}

pub(super) fn accumulate(
    normal: &mut [[f32; PARAMS]; PARAMS],
    rhs: &mut [f32; PARAMS],
    weight_sum: &mut f32,
    field_ut: [f32; 3],
    origin: [f32; 3],
) {
    let x = (field_ut[0] - origin[0]) / FIT_INPUT_SCALE_UT;
    let y = (field_ut[1] - origin[1]) / FIT_INPUT_SCALE_UT;
    let z = (field_ut[2] - origin[2]) / FIT_INPUT_SCALE_UT;
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

    for row in 0..PARAMS {
        rhs[row] += feature[row];
        for col in 0..PARAMS {
            normal[row][col] += feature[row] * feature[col];
        }
    }
    *weight_sum += 1.0;
}

pub(super) fn fit_model(
    normal: &[[f32; PARAMS]; PARAMS],
    rhs: &[f32; PARAMS],
    weight_sum: f32,
    fit_origin_ut: Option<[f32; 3]>,
    min: [f32; 3],
    max: [f32; 3],
) -> Option<Model> {
    if weight_sum < CALIBRATION_MIN_FIT_SAMPLES as f32 * 0.75 {
        return None;
    }

    let inverse_weight = 1.0 / weight_sum;
    let mut augmented = [[0.0; PARAMS + 1]; PARAMS];
    for row in 0..PARAMS {
        for col in 0..PARAMS {
            augmented[row][col] = normal[row][col] * inverse_weight;
        }
        augmented[row][PARAMS] = rhs[row] * inverse_weight;
    }
    let parameters = solve_linear_9(augmented)?;

    let rms = algebraic_rms(parameters, normal, rhs, weight_sum);
    if !rms.is_finite() || rms > MAX_ALGEBRAIC_RMS {
        return None;
    }

    let q = [
        [parameters[0], parameters[3], parameters[4]],
        [parameters[3], parameters[1], parameters[5]],
        [parameters[4], parameters[5], parameters[2]],
    ];
    let linear = [parameters[6], parameters[7], parameters[8]];
    let q_center = solve_linear_3(q, linear)?;
    let center_normalized = [-0.5 * q_center[0], -0.5 * q_center[1], -0.5 * q_center[2]];

    let scale = 1.0 + dot3(center_normalized, matrix_vector(q, center_normalized));
    if !scale.is_finite() || abs_f32(scale) < QUADRIC_SCALE_EPSILON {
        return None;
    }

    let mut shape = [[0.0; 3]; 3];
    for row in 0..3 {
        for col in 0..3 {
            shape[row][col] = q[row][col] / scale;
        }
    }

    let (eigenvalues, eigenvectors) = symmetric_eigen_3(shape)?;
    let mut min_eigen = f32::MAX;
    let mut max_eigen = 0.0f32;
    let mut sqrt_eigen = [0.0; 3];
    for axis in 0..3 {
        let eigenvalue = eigenvalues[axis];
        if !eigenvalue.is_finite() || eigenvalue <= SHAPE_EIGEN_EPSILON {
            return None;
        }
        min_eigen = min_f32(min_eigen, eigenvalue);
        max_eigen = max_f32(max_eigen, eigenvalue);
        sqrt_eigen[axis] = sqrt_approx(eigenvalue);

        let radius_ut = FIT_INPUT_SCALE_UT / sqrt_eigen[axis];
        if !(MIN_ELLIPSOID_RADIUS_UT..=MAX_ELLIPSOID_RADIUS_UT).contains(&radius_ut) {
            return None;
        }
    }
    if max_eigen / min_eigen > MAX_SHAPE_EIGEN_RATIO {
        return None;
    }

    let origin = fit_origin_ut?;
    let center_ut = [
        origin[0] + center_normalized[0] * FIT_INPUT_SCALE_UT,
        origin[1] + center_normalized[1] * FIT_INPUT_SCALE_UT,
        origin[2] + center_normalized[2] * FIT_INPUT_SCALE_UT,
    ];
    if !center_is_covered(center_ut, min, max) {
        return None;
    }

    // Symmetric square root: V * sqrt(D) * V^T. Multiplying by the target
    // radius and undoing input normalization gives a direct raw-uT -> uT
    // correction matrix without an arbitrary coordinate rotation.
    let mut correction = [[0.0; 3]; 3];
    let output_scale = CALIBRATED_FIELD_RADIUS_UT / FIT_INPUT_SCALE_UT;
    for row in 0..3 {
        for col in 0..3 {
            let mut value = 0.0;
            for axis in 0..3 {
                value += eigenvectors[row][axis] * sqrt_eigen[axis] * eigenvectors[col][axis];
            }
            correction[row][col] = value * output_scale;
            if !correction[row][col].is_finite() {
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
    for axis in 0..3 {
        let span = max[axis] - min[axis];
        let margin = 0.10 * span + 5.0;
        if center_ut[axis] < min[axis] - margin || center_ut[axis] > max[axis] + margin {
            return false;
        }
    }
    true
}

fn algebraic_rms(
    parameters: [f32; PARAMS],
    normal: &[[f32; PARAMS]; PARAMS],
    rhs: &[f32; PARAMS],
    weight_sum: f32,
) -> f32 {
    let mut quadratic = 0.0;
    let mut linear = 0.0;
    for row in 0..PARAMS {
        linear += parameters[row] * rhs[row];
        for col in 0..PARAMS {
            quadratic += parameters[row] * normal[row][col] * parameters[col];
        }
    }
    let sse = max_f32(0.0, quadratic - 2.0 * linear + weight_sum);
    sqrt_approx(sse / weight_sum)
}

fn vector_length(value: [f32; 3]) -> f32 {
    sqrt_approx(dot3(value, value))
}
