//! Ellipsoid fitting and candidate validation for BMM150 calibration.
//!
//! The fit is algebraic: it looks for the nine parameters `p` of a quadric
//! (a surface given by a quadratic equation, such as an ellipsoid) so that
//! `p · feature = 1` holds as closely as possible for every sample (least
//! squares). Then it checks that the result is a plausible ellipsoid.

use super::math::{PARAMS, solve_linear, symmetric_eigen_3};
use crate::imu::vec3;

/// Samples needed in the fit before a fit is attempted: more than ten per
/// fitted parameter (there are nine). The per-bin limit keeps these samples
/// balanced between directions.
pub const MIN_FIT_SAMPLES: u32 = 96;

// A candidate is checked on new samples that were not part of its fit, and
// these samples must come from several directions. At 30 Hz, the maximum
// gives the user about ten seconds to cover six directions.
/// New samples a candidate must see before it is judged.
const VALIDATION_MIN_SAMPLES: u16 = 24;
/// Distinct direction bins those samples must cover.
const VALIDATION_MIN_BINS: u32 = 6;
/// Samples after which a candidate that is still pending is rejected.
const VALIDATION_MAX_SAMPLES: u16 = 300;
/// Largest accepted RMS (root mean square) of the relative error of the
/// corrected field strength. 0.15 means 15 % away from
/// `CALIBRATED_FIELD_RADIUS_UT`.
const MAX_VALIDATION_RMS_RELATIVE_ERROR: f32 = 0.15;
/// A sample whose corrected strength is more than 35 % away from
/// `CALIBRATED_FIELD_RADIUS_UT` counts as bad.
const MAX_VALIDATION_SINGLE_RELATIVE_ERROR: f32 = 0.35;
/// Bad samples a candidate may have. One more rejects it at once.
const MAX_VALIDATION_BAD_SAMPLES: u8 = 6;

/// Target strength, in uT, of every corrected field.
pub const CALIBRATED_FIELD_RADIUS_UT: f32 = 50.0;
/// The fit works on fields, in uT, divided by this scale. So the quadratic
/// and the linear terms have similar sizes, and `f32` stays precise enough.
const FIT_INPUT_SCALE_UT: f32 = 256.0;
/// Limit for the right-hand side of the centered quadric equation.
///
/// With its center `c` moved to zero, the fitted quadric is
/// `x · Q x = 1 + c · Q c`. The fit divides `Q` by that right-hand side. When
/// the right-hand side is closer to zero than this, the fit is rejected.
const QUADRIC_SCALE_EPSILON: f32 = 1.0e-6;
/// Every eigenvalue of the fitted shape matrix must be larger than this,
/// because an ellipsoid has three positive eigenvalues. Each eigenvalue is
/// `1 / semi-axis²`, with the semi-axis in scaled units.
const SHAPE_EIGEN_EPSILON: f32 = 1.0e-6;
/// Largest accepted ratio of the largest to the smallest shape eigenvalue.
/// Eigenvalues are `1 / semi-axis²`, so 400 allows one semi-axis to be at
/// most 20 times as long as another.
const MAX_SHAPE_EIGEN_RATIO: f32 = 400.0;
/// Longest accepted ellipsoid semi-axis (half of an axis), in uT.
const MAX_ELLIPSOID_RADIUS_UT: f32 = 300.0;
/// Shortest accepted ellipsoid semi-axis, in uT.
const MIN_ELLIPSOID_RADIUS_UT: f32 = 10.0;
/// Largest accepted RMS residual of the fit equation
/// `parameters · feature = 1` over the samples. The residual is the
/// difference between the two sides. It has no unit.
const MAX_ALGEBRAIC_RMS: f32 = 0.15;
/// The fitted center must lie between the observed extrema, widened by a
/// margin. This is the part of the margin relative to the span of the axis.
const CENTER_MARGIN_FRACTION: f32 = 0.10;
/// Fixed part of that margin, in uT, added to the relative part.
const CENTER_MARGIN_UT: f32 = 5.0;

/// Hard-iron offset and soft-iron correction matrix.
#[derive(Clone, Copy)]
pub struct Model {
    /// Hard-iron offset: the ellipsoid center, in raw uT.
    pub center_ut: [f32; 3],
    /// Soft-iron matrix, as rows. It is applied after the offset is removed.
    /// It maps the ellipsoid onto a sphere of radius
    /// `CALIBRATED_FIELD_RADIUS_UT`.
    correction: [[f32; 3]; 3],
}

impl Model {
    /// Correct a raw field: remove the hard-iron offset, then undo the
    /// soft-iron distortion. In and out in uT.
    pub fn apply(&self, field_ut: [f32; 3]) -> [f32; 3] {
        vec3::matrix_vector(self.correction, vec3::sub(field_ut, self.center_ut))
    }
}

/// A fitted model that is checked on new samples before it is trusted.
#[derive(Clone, Copy)]
pub struct Candidate {
    /// The model under test.
    model: Model,
    /// Validation samples seen so far.
    samples: u16,
    /// Bit mask of the direction bins, around the model's center, that the
    /// validation samples covered.
    direction_bins: u32,
    /// Sum of the squared relative strength errors, for the RMS.
    relative_error_squared_sum: f32,
    /// Samples whose relative error was larger than
    /// `MAX_VALIDATION_SINGLE_RELATIVE_ERROR` or was not a finite number.
    bad_samples: u8,
}

impl Candidate {
    /// A candidate for `model` that has seen no validation samples yet.
    pub const fn new(model: Model) -> Self {
        Self {
            model,
            samples: 0,
            direction_bins: 0,
            relative_error_squared_sum: 0.0,
            bad_samples: 0,
        }
    }

    /// Fraction of the validation requirement met so far, 0 to 1.
    pub fn progress(self) -> f32 {
        let samples = f32::from(self.samples) / f32::from(VALIDATION_MIN_SAMPLES);
        let bins = self.direction_bins.count_ones() as f32 / VALIDATION_MIN_BINS as f32;
        samples.min(bins).clamp(0.0, 1.0)
    }
}

/// What one validation sample decided about a candidate.
pub enum Validation {
    /// More samples are needed; keep validating this updated candidate.
    Pending(Candidate),
    /// The candidate passed; this model may be used.
    Accepted(Model),
    /// The candidate failed; collect a new set of samples.
    Rejected,
}

/// Check a candidate on one new raw field, in uT.
///
/// The candidate is judged once it has seen `VALIDATION_MIN_SAMPLES` samples
/// in `VALIDATION_MIN_BINS` direction bins. It is rejected earlier after too
/// many bad samples, or after `VALIDATION_MAX_SAMPLES` samples.
pub fn validate_candidate(mut candidate: Candidate, field_ut: [f32; 3]) -> Validation {
    // The direction bins are measured around the fitted center, because it
    // is the best estimate of the real center.
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

/// The direction bin, 0 to 23, of a field relative to `origin_ut`. The bins
/// show how well the samples cover all directions.
///
/// There are 24 bins: 6 cube faces times 4 quadrants. The face is the axis
/// with the largest absolute value and its sign: 0 and 1 for +x and -x, 2
/// and 3 for y, 4 and 5 for z. The quadrant comes from the signs of the two
/// other axes. The bin is `face * 4 + quadrant`.
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

/// The normal equations of the least-squares ellipsoid fit, as sums over
/// the samples.
///
/// The best parameters `p` solve `matrix * p = rhs`.
pub struct NormalEquations {
    /// Sum over the samples of `feature[i] * feature[j]`: the matrix of the
    /// least-squares problem.
    matrix: [[f32; PARAMS]; PARAMS],
    /// Sum over the samples of `feature[i]`: the right-hand side. It is this
    /// simple sum because each sample should satisfy `parameters · feature = 1`.
    rhs: [f32; PARAMS],
    /// Number of samples added; every sample has weight 1.
    weight_sum: f32,
}

impl NormalEquations {
    /// Sums of no samples.
    pub const fn new() -> Self {
        Self {
            matrix: [[0.0; PARAMS]; PARAMS],
            rhs: [0.0; PARAMS],
            weight_sum: 0.0,
        }
    }

    /// Add one raw field, in uT, to the sums.
    ///
    /// First `origin_ut` is subtracted and the result is divided by
    /// `FIT_INPUT_SCALE_UT`. The feature vector of the scaled field is
    /// x², y², z², 2xy, 2xz, 2yz, x, y, z.
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

    /// RMS of the residuals `parameters · feature - 1` over all samples,
    /// computed from the sums.
    fn rms(&self, parameters: [f32; PARAMS]) -> f32 {
        let mut quadratic = 0.0;
        let mut linear = 0.0;
        for (row, (matrix_row, rhs)) in self.matrix.iter().zip(&self.rhs).enumerate() {
            linear += parameters[row] * rhs;
            for (cell, parameter) in matrix_row.iter().zip(parameters) {
                quadratic += parameters[row] * cell * parameter;
            }
        }
        // Sum of squared errors: Σ (p · f - 1)² = pᵀ M p - 2 p · rhs + n.
        // Rounding can make it slightly negative, so it is limited to 0.
        let sse = (quadratic - 2.0 * linear + self.weight_sum).max(0.0);
        libm::sqrtf(sse / self.weight_sum)
    }
}

/// Fit an ellipsoid to the collected samples.
///
/// `origin_ut` is the point the samples were measured from. `min` and `max`
/// are the observed extrema; the fitted center must lie near them.
///
/// Returns `None` when there are too few samples, when a linear system
/// cannot be solved, or when the result is not a plausible ellipsoid (see
/// the limits above).
pub fn fit_model(
    equations: &NormalEquations,
    origin_ut: [f32; 3],
    min: [f32; 3],
    max: [f32; 3],
) -> Option<Model> {
    if equations.weight_sum < MIN_FIT_SAMPLES as f32 {
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

    // The correction is the symmetric square root of the shape matrix:
    // V * sqrt(D) * Vᵀ. V has the eigenvectors as columns, D the eigenvalues
    // on its diagonal. It is scaled to the target radius and undoes the input
    // scale. So it maps a raw field in uT, minus the center, directly to uT.
    // Being symmetric, it does not add a rotation.
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

/// Whether the fitted center lies within the observed extrema. On each axis
/// the extrema are widened by `CENTER_MARGIN_FRACTION` of the span plus
/// `CENTER_MARGIN_UT`.
fn center_is_covered(center_ut: [f32; 3], min: [f32; 3], max: [f32; 3]) -> bool {
    (0..3).all(|axis| {
        let margin = CENTER_MARGIN_FRACTION * (max[axis] - min[axis]) + CENTER_MARGIN_UT;
        (min[axis] - margin..=max[axis] + margin).contains(&center_ut[axis])
    })
}
