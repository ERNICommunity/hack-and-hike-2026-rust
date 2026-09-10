//! Runtime hard/soft-iron calibration for the BMM150 magnetic field.
//!
//! This submodule is deliberately independent from the BMM150 transport/factory
//! compensation logic. It fits Bosch-compensated body-frame samples to a
//! general ellipsoid and converts that fit into a symmetric 3x3 correction
//! matrix. Learning is coverage-balanced so repeatedly waving the device through
//! the same few orientations cannot dominate the fit, and every provisional fit
//! must predict fresh samples before it is accepted.
//!
//! The fitted quadric is
//!
//!     x^T Q x + l^T x = 1
//!
//! on inputs normalized by [`FIT_INPUT_SCALE_UT`]. Its hard-iron center is
//! `c = -0.5 Q^-1 l`. After translation, the ellipsoid shape is
//! `S = Q / (1 + c^T Q c)`. A Jacobi eigensolver computes the symmetric
//! positive-definite square root of `S`; using the symmetric root rather than a
//! Cholesky whitening factor avoids introducing an arbitrary rotation into the
//! magnetic heading frame.

const PARAMS: usize = 9;
const AUGMENTED: usize = PARAMS + 1;

/// Compensated BMM150 fields inside the CoreS3 enclosure can be far larger than
/// the Earth's field before hard-iron removal. Keep the learning window broad,
/// but reject near-zero/overflow-like samples.
const LEARNING_FIELD_MIN_UT: f32 = 5.0;
const LEARNING_FIELD_MAX_UT: f32 = 4000.0;

/// Post-calibration magnitude gate. The calibration intentionally normalizes
/// the accepted ellipsoid to 50 uT, so a much wider 5..150 uT window hid bad
/// fits. This still leaves generous room for noise and transient disturbances.
pub const GOOD_FIELD_MIN_UT: f32 = 25.0;
pub const GOOD_FIELD_MAX_UT: f32 = 80.0;

const CALIBRATED_FIELD_RADIUS_UT: f32 = 50.0;
const CALIBRATION_TARGET_SPAN_UT: f32 = 35.0;
const ORIGIN_WARMUP_SAMPLES: u32 = 60;
const ORIGIN_MIN_SPAN_UT: f32 = 20.0;

// Divide the sphere into six dominant-axis faces, each split into four
// quadrants. Requiring many of these 24 sectors is substantially stronger than
// checking only independent X/Y/Z extrema, while remaining tiny and allocation
// free. Samples in a saturated sector are ignored by the least-squares history
// so lingering in one pose cannot overwhelm rarer orientations.
const DIRECTION_BIN_COUNT: usize = 24;
const MIN_DIRECTION_BINS: u32 = 18;
const MIN_DIRECTION_FACES: u32 = 6;
const MAX_SAMPLES_PER_DIRECTION_BIN: u8 = 32;
const CALIBRATION_MIN_FIT_SAMPLES: u32 = 180;
const REFIT_INTERVAL_SAMPLES: u16 = 24;
const MAX_FAILED_FITS_BEFORE_RESTART: u8 = 6;

// A provisional fit must generalize to fresh measurements before becoming
// Ready. This geometric radial validation is much more meaningful than the
// algebraic fitting residual alone and prevents a numerically valid but poorly
// covered ellipsoid from freezing permanently.
const CANDIDATE_VALIDATION_MIN_SAMPLES: u16 = 48;
const CANDIDATE_VALIDATION_MIN_BINS: u32 = 8;
const CANDIDATE_VALIDATION_MAX_SAMPLES: u16 = 120;
const MAX_CANDIDATE_RMS_RELATIVE_ERROR: f32 = 0.15;
const MAX_CANDIDATE_SINGLE_RELATIVE_ERROR: f32 = 0.35;
const MAX_CANDIDATE_BAD_SAMPLES: u8 = 6;

// Fitting the quadratic directly around the device's ~1-2 mT hard-iron offset
// makes the f32 normal equations badly conditioned. First estimate a fixed local
// origin from warm-up extrema, then fit the ellipsoid in coordinates around that
// origin. Typical local coordinates are a few hundred uT, so 1024 uT maps them
// near unity without losing range.
const FIT_INPUT_SCALE_UT: f32 = 1024.0;
const SOLVER_RELATIVE_PIVOT_EPSILON: f32 = 1.0e-6;
const QUADRIC_SCALE_EPSILON: f32 = 1.0e-6;
const SHAPE_EIGEN_EPSILON: f32 = 1.0e-6;
const MAX_SHAPE_EIGEN_RATIO: f32 = 400.0;
const MAX_ELLIPSOID_RADIUS_UT: f32 = 300.0;
const MIN_ELLIPSOID_RADIUS_UT: f32 = 10.0;
const MAX_ALGEBRAIC_RMS: f32 = 0.15;
const JACOBI_ROTATIONS: usize = 18;

#[derive(Clone, Copy)]
struct Model {
    center_ut: [f32; 3],
    correction: [[f32; 3]; 3],
}

impl Model {
    fn apply(&self, field_ut: [f32; 3]) -> [f32; 3] {
        let centered = [
            field_ut[0] - self.center_ut[0],
            field_ut[1] - self.center_ut[1],
            field_ut[2] - self.center_ut[2],
        ];
        matrix_vector(self.correction, centered)
    }
}

#[derive(Clone, Copy)]
struct Candidate {
    model: Model,
    samples: u16,
    direction_bins: u32,
    relative_error_squared_sum: f32,
    bad_samples: u8,
}

impl Candidate {
    const fn new(model: Model) -> Self {
        Self {
            model,
            samples: 0,
            direction_bins: 0,
            relative_error_squared_sum: 0.0,
            bad_samples: 0,
        }
    }
}

/// Allocation-free online full-ellipsoid magnetometer calibration.
///
/// The fitter accumulates 9x9 normal equations for the general quadratic terms
/// `x^2, y^2, z^2, 2xy, 2xz, 2yz, x, y, z`, but only up to a bounded number of
/// samples per 3-D direction sector. A candidate model is then checked against
/// fresh, directionally diverse samples before it is frozen.
pub struct Calibration {
    normal: [[f32; PARAMS]; PARAMS],
    rhs: [f32; PARAMS],
    min: [f32; 3],
    max: [f32; 3],
    samples: u32,
    fit_origin_ut: Option<[f32; 3]>,
    fit_samples: u32,
    weight_sum: f32,
    samples_since_fit: u16,
    direction_bins: u32,
    direction_faces: u8,
    direction_bin_samples: [u8; DIRECTION_BIN_COUNT],
    failed_fits: u8,
    candidate: Option<Candidate>,
    model: Option<Model>,
}

impl Calibration {
    pub const fn new() -> Self {
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
            direction_bins: 0,
            direction_faces: 0,
            direction_bin_samples: [0; DIRECTION_BIN_COUNT],
            failed_fits: 0,
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
    /// A repeatedly invalid history is discarded automatically instead of
    /// remaining indefinitely at 99%. Once a validated model is accepted it is
    /// frozen so external magnetic disturbances cannot be learned as enclosure
    /// hard/soft iron.
    pub fn observe(&mut self, field_ut: [f32; 3]) {
        if self.model.is_some() || !raw_sample_is_plausible(field_ut) {
            return;
        }

        for axis in 0..3 {
            self.min[axis] = min_f32(self.min[axis], field_ut[axis]);
            self.max[axis] = max_f32(self.max[axis], field_ut[axis]);
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

        let origin = self.fit_origin_ut.unwrap_or([0.0; 3]);
        let direction_bin = direction_bin(field_ut, origin);

        if self.candidate.is_some() {
            self.validate_candidate(field_ut, direction_bin);
            if self.model.is_some() || self.fit_origin_ut.is_none() {
                return;
            }
        }

        self.direction_bins |= 1u32 << direction_bin;
        self.direction_faces |= 1u8 << (direction_bin / 4);

        if self.direction_bin_samples[direction_bin] < MAX_SAMPLES_PER_DIRECTION_BIN {
            self.direction_bin_samples[direction_bin] += 1;
            self.accumulate(field_ut);
            self.fit_samples = self.fit_samples.saturating_add(1);
            self.samples_since_fit = self.samples_since_fit.saturating_add(1);
        }

        if self.candidate.is_none()
            && self.has_minimum_coverage()
            && self.samples_since_fit >= REFIT_INTERVAL_SAMPLES
        {
            self.samples_since_fit = 0;
            match self.fit_model() {
                Some(model) => self.candidate = Some(Candidate::new(model)),
                None => self.record_failed_fit(),
            }
        }
    }

    pub fn is_ready(&self) -> bool {
        self.model.is_some()
    }

    /// Report meaningful phase progress rather than sample-count optimism.
    ///
    /// 0..20: origin warm-up
    /// 20..85: balanced 3-D fitting coverage
    /// 85..99: fresh-sample candidate validation
    /// 100: validated and frozen
    ///
    /// A rejected fit returns to the coverage phase (or restarts from zero after
    /// repeated failures), so 99% can no longer mean "stuck indefinitely".
    pub fn progress_percent(&self) -> u8 {
        if self.model.is_some() {
            return 100;
        }
        if self.samples == 0 {
            return 0;
        }

        if self.fit_origin_ut.is_none() {
            let sample_progress = clamp_f32(
                self.samples as f32 / ORIGIN_WARMUP_SAMPLES as f32,
                0.0,
                1.0,
            );
            let span_progress = clamp_f32(
                self.minimum_span() / ORIGIN_MIN_SPAN_UT,
                0.0,
                1.0,
            );
            return (20.0 * min_f32(sample_progress, span_progress)) as u8;
        }

        let sample_progress = clamp_f32(
            self.fit_samples as f32 / CALIBRATION_MIN_FIT_SAMPLES as f32,
            0.0,
            1.0,
        );
        let span_progress = clamp_f32(
            self.minimum_span() / CALIBRATION_TARGET_SPAN_UT,
            0.0,
            1.0,
        );
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
            let validation_samples = clamp_f32(
                candidate.samples as f32 / CANDIDATE_VALIDATION_MIN_SAMPLES as f32,
                0.0,
                1.0,
            );
            let validation_bins = clamp_f32(
                bit_count_u32(candidate.direction_bins) as f32
                    / CANDIDATE_VALIDATION_MIN_BINS as f32,
                0.0,
                1.0,
            );
            let validation_progress = min_f32(validation_samples, validation_bins);
            return (85.0 + 14.0 * validation_progress) as u8;
        }

        (coverage_percent as u8).min(85)
    }

    pub fn apply(&self, field_ut: [f32; 3]) -> [f32; 3] {
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

    fn minimum_span(&self) -> f32 {
        if self.samples == 0 {
            return 0.0;
        }
        min_f32(
            self.max[0] - self.min[0],
            min_f32(self.max[1] - self.min[1], self.max[2] - self.min[2]),
        )
    }

    fn validate_candidate(&mut self, field_ut: [f32; 3], direction_bin: usize) {
        let Some(mut candidate) = self.candidate else {
            return;
        };

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
            let rms = sqrt_approx(
                candidate.relative_error_squared_sum / candidate.samples.max(1) as f32,
            );
            if rms.is_finite()
                && rms <= MAX_CANDIDATE_RMS_RELATIVE_ERROR
                && candidate.bad_samples <= MAX_CANDIDATE_BAD_SAMPLES
            {
                self.model = Some(candidate.model);
                self.candidate = None;
                return;
            }

            self.candidate = None;
            self.record_failed_fit();
            return;
        }

        if candidate.samples >= CANDIDATE_VALIDATION_MAX_SAMPLES
            || candidate.bad_samples > MAX_CANDIDATE_BAD_SAMPLES
        {
            self.candidate = None;
            self.record_failed_fit();
            return;
        }

        self.candidate = Some(candidate);
    }

    fn record_failed_fit(&mut self) {
        self.failed_fits = self.failed_fits.saturating_add(1);
        if self.failed_fits >= MAX_FAILED_FITS_BEFORE_RESTART {
            self.restart_learning();
        } else {
            // Any samples gathered while a candidate was being checked have
            // improved the training history. Permit a prompt refit rather than
            // waiting for another full interval.
            self.samples_since_fit = REFIT_INTERVAL_SAMPLES;
        }
    }

    fn restart_learning(&mut self) {
        *self = Self::new();
    }

    fn accumulate(&mut self, field_ut: [f32; 3]) {
        let origin = self.fit_origin_ut.unwrap_or([0.0; 3]);
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
            self.rhs[row] += feature[row];
            for col in 0..PARAMS {
                self.normal[row][col] += feature[row] * feature[col];
            }
        }
        self.weight_sum += 1.0;
    }

    fn fit_model(&self) -> Option<Model> {
        if self.weight_sum < CALIBRATION_MIN_FIT_SAMPLES as f32 * 0.75 {
            return None;
        }

        let inverse_weight = 1.0 / self.weight_sum;
        let mut augmented = [[0.0; AUGMENTED]; PARAMS];
        for row in 0..PARAMS {
            for col in 0..PARAMS {
                augmented[row][col] = self.normal[row][col] * inverse_weight;
            }
            augmented[row][PARAMS] = self.rhs[row] * inverse_weight;
        }
        let parameters = solve_linear_9(augmented)?;

        let rms = algebraic_rms(parameters, &self.normal, &self.rhs, self.weight_sum);
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
        let center_normalized = [
            -0.5 * q_center[0],
            -0.5 * q_center[1],
            -0.5 * q_center[2],
        ];

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

        let origin = self.fit_origin_ut?;
        let center_ut = [
            origin[0] + center_normalized[0] * FIT_INPUT_SCALE_UT,
            origin[1] + center_normalized[1] * FIT_INPUT_SCALE_UT,
            origin[2] + center_normalized[2] * FIT_INPUT_SCALE_UT,
        ];
        if !self.center_is_covered(center_ut) {
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
                    value += eigenvectors[row][axis]
                        * sqrt_eigen[axis]
                        * eigenvectors[col][axis];
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

    fn center_is_covered(&self, center_ut: [f32; 3]) -> bool {
        for axis in 0..3 {
            let span = self.max[axis] - self.min[axis];
            let margin = 0.10 * span + 5.0;
            if center_ut[axis] < self.min[axis] - margin
                || center_ut[axis] > self.max[axis] + margin
            {
                return false;
            }
        }
        true
    }
}

fn direction_bin(field_ut: [f32; 3], origin_ut: [f32; 3]) -> usize {
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

fn bit_count_u32(mut value: u32) -> u32 {
    let mut count = 0;
    while value != 0 {
        value &= value - 1;
        count += 1;
    }
    count
}

fn bit_count_u8(mut value: u8) -> u32 {
    let mut count = 0;
    while value != 0 {
        value &= value - 1;
        count += 1;
    }
    count
}

fn raw_sample_is_plausible(field_ut: [f32; 3]) -> bool {
    let strength = vector_length(field_ut);
    (LEARNING_FIELD_MIN_UT..=LEARNING_FIELD_MAX_UT).contains(&strength)
}

pub fn vector_length(value: [f32; 3]) -> f32 {
    sqrt_approx(dot3(value, value))
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

fn solve_linear_9(mut augmented: [[f32; AUGMENTED]; PARAMS]) -> Option<[f32; PARAMS]> {
    let mut matrix_max = 0.0f32;
    for row in 0..PARAMS {
        for col in 0..PARAMS {
            matrix_max = max_f32(matrix_max, abs_f32(augmented[row][col]));
        }
    }
    if matrix_max <= 0.0 {
        return None;
    }
    let pivot_epsilon = matrix_max * SOLVER_RELATIVE_PIVOT_EPSILON;

    for col in 0..PARAMS {
        let mut pivot_row = col;
        let mut pivot_abs = abs_f32(augmented[col][col]);
        for row in (col + 1)..PARAMS {
            let candidate = abs_f32(augmented[row][col]);
            if candidate > pivot_abs {
                pivot_abs = candidate;
                pivot_row = row;
            }
        }
        if pivot_abs <= pivot_epsilon {
            return None;
        }
        if pivot_row != col {
            augmented.swap(pivot_row, col);
        }

        let pivot = augmented[col][col];
        for index in col..AUGMENTED {
            augmented[col][index] /= pivot;
        }

        for row in 0..PARAMS {
            if row == col {
                continue;
            }
            let factor = augmented[row][col];
            if factor == 0.0 {
                continue;
            }
            for index in col..AUGMENTED {
                augmented[row][index] -= factor * augmented[col][index];
            }
        }
    }

    let mut result = [0.0; PARAMS];
    for row in 0..PARAMS {
        result[row] = augmented[row][PARAMS];
        if !result[row].is_finite() {
            return None;
        }
    }
    Some(result)
}

fn solve_linear_3(matrix: [[f32; 3]; 3], rhs: [f32; 3]) -> Option<[f32; 3]> {
    let mut augmented = [[0.0; 4]; 3];
    let mut matrix_max = 0.0f32;
    for row in 0..3 {
        for col in 0..3 {
            augmented[row][col] = matrix[row][col];
            matrix_max = max_f32(matrix_max, abs_f32(matrix[row][col]));
        }
        augmented[row][3] = rhs[row];
    }
    if matrix_max <= 0.0 {
        return None;
    }
    let pivot_epsilon = matrix_max * SOLVER_RELATIVE_PIVOT_EPSILON;

    for col in 0..3 {
        let mut pivot_row = col;
        let mut pivot_abs = abs_f32(augmented[col][col]);
        for row in (col + 1)..3 {
            let candidate = abs_f32(augmented[row][col]);
            if candidate > pivot_abs {
                pivot_abs = candidate;
                pivot_row = row;
            }
        }
        if pivot_abs <= pivot_epsilon {
            return None;
        }
        if pivot_row != col {
            augmented.swap(pivot_row, col);
        }

        let pivot = augmented[col][col];
        for index in col..4 {
            augmented[col][index] /= pivot;
        }
        for row in 0..3 {
            if row == col {
                continue;
            }
            let factor = augmented[row][col];
            for index in col..4 {
                augmented[row][index] -= factor * augmented[col][index];
            }
        }
    }

    Some([augmented[0][3], augmented[1][3], augmented[2][3]])
}

/// Jacobi diagonalization for a real symmetric 3x3 matrix.
///
/// The fixed rotation count gives deterministic cost on the embedded target and
/// is ample for a 3x3 matrix. The returned eigenvectors are columns of `vectors`.
fn symmetric_eigen_3(mut matrix: [[f32; 3]; 3]) -> Option<([f32; 3], [[f32; 3]; 3])> {
    let mut vectors = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

    for _ in 0..JACOBI_ROTATIONS {
        let (p, q, offdiag) = largest_offdiag(matrix);
        if offdiag <= 1.0e-7 {
            break;
        }

        let app = matrix[p][p];
        let aqq = matrix[q][q];
        let apq = matrix[p][q];
        if abs_f32(apq) <= 1.0e-12 {
            continue;
        }

        let tau = (aqq - app) / (2.0 * apq);
        let t = if tau >= 0.0 {
            1.0 / (tau + sqrt_approx(1.0 + tau * tau))
        } else {
            -1.0 / (-tau + sqrt_approx(1.0 + tau * tau))
        };
        let cosine = 1.0 / sqrt_approx(1.0 + t * t);
        let sine = t * cosine;

        for k in 0..3 {
            if k == p || k == q {
                continue;
            }
            let akp = matrix[k][p];
            let akq = matrix[k][q];
            let new_kp = cosine * akp - sine * akq;
            let new_kq = sine * akp + cosine * akq;
            matrix[k][p] = new_kp;
            matrix[p][k] = new_kp;
            matrix[k][q] = new_kq;
            matrix[q][k] = new_kq;
        }

        let c2 = cosine * cosine;
        let s2 = sine * sine;
        let two_sc = 2.0 * sine * cosine;
        matrix[p][p] = c2 * app - two_sc * apq + s2 * aqq;
        matrix[q][q] = s2 * app + two_sc * apq + c2 * aqq;
        matrix[p][q] = 0.0;
        matrix[q][p] = 0.0;

        for row in 0..3 {
            let vip = vectors[row][p];
            let viq = vectors[row][q];
            vectors[row][p] = cosine * vip - sine * viq;
            vectors[row][q] = sine * vip + cosine * viq;
        }
    }

    let eigenvalues = [matrix[0][0], matrix[1][1], matrix[2][2]];
    if eigenvalues.iter().any(|value| !value.is_finite()) {
        return None;
    }
    Some((eigenvalues, vectors))
}

fn largest_offdiag(matrix: [[f32; 3]; 3]) -> (usize, usize, f32) {
    let a01 = abs_f32(matrix[0][1]);
    let a02 = abs_f32(matrix[0][2]);
    let a12 = abs_f32(matrix[1][2]);
    if a01 >= a02 && a01 >= a12 {
        (0, 1, a01)
    } else if a02 >= a12 {
        (0, 2, a02)
    } else {
        (1, 2, a12)
    }
}

fn matrix_vector(matrix: [[f32; 3]; 3], vector: [f32; 3]) -> [f32; 3] {
    [
        matrix[0][0] * vector[0] + matrix[0][1] * vector[1] + matrix[0][2] * vector[2],
        matrix[1][0] * vector[0] + matrix[1][1] * vector[1] + matrix[1][2] * vector[2],
        matrix[2][0] * vector[0] + matrix[2][1] * vector[1] + matrix[2][2] * vector[2],
    ]
}

fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn min_f32(a: f32, b: f32) -> f32 {
    if a < b { a } else { b }
}

fn max_f32(a: f32, b: f32) -> f32 {
    if a > b { a } else { b }
}

fn clamp_f32(value: f32, min: f32, max: f32) -> f32 {
    if value < min {
        min
    } else if value > max {
        max
    } else {
        value
    }
}

fn abs_f32(value: f32) -> f32 {
    if value < 0.0 { -value } else { value }
}

fn sqrt_approx(value: f32) -> f32 {
    if value <= 0.0 {
        return 0.0;
    }
    let mut estimate = if value > 1.0 { value } else { 1.0 };
    for _ in 0..10 {
        estimate = 0.5 * (estimate + value / estimate);
    }
    estimate
}
