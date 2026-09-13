//! Fixed-size numerical helpers used only by BMM150 calibration.

pub(super) const PARAMS: usize = 9;
const SOLVER_RELATIVE_PIVOT_EPSILON: f32 = 1.0e-6;
const JACOBI_ROTATIONS: usize = 18;

/// Solve `matrix * x = rhs` by Gauss-Jordan elimination with partial pivoting.
///
/// Returns `None` when the system is singular or numerically unstable.
pub(super) fn solve_linear<const N: usize>(
    mut matrix: [[f32; N]; N],
    mut rhs: [f32; N],
) -> Option<[f32; N]> {
    let matrix_max = matrix
        .iter()
        .flatten()
        .fold(0.0f32, |max, value| max_f32(max, abs_f32(*value)));
    if matrix_max <= 0.0 {
        return None;
    }
    let pivot_epsilon = matrix_max * SOLVER_RELATIVE_PIVOT_EPSILON;

    for col in 0..N {
        let pivot_row = matrix
            .iter()
            .enumerate()
            .skip(col)
            .fold(col, |best, (row, values)| {
                if abs_f32(values[col]) > abs_f32(matrix[best][col]) {
                    row
                } else {
                    best
                }
            });
        if abs_f32(matrix[pivot_row][col]) <= pivot_epsilon {
            return None;
        }
        matrix.swap(pivot_row, col);
        rhs.swap(pivot_row, col);

        let pivot = matrix[col][col];
        for value in &mut matrix[col] {
            *value /= pivot;
        }
        rhs[col] /= pivot;

        let pivot_values = matrix[col];
        let pivot_rhs = rhs[col];
        for (row, (values, value_rhs)) in matrix.iter_mut().zip(&mut rhs).enumerate() {
            if row == col {
                continue;
            }
            let factor = values[col];
            if factor == 0.0 {
                continue;
            }
            for (value, pivot_value) in values.iter_mut().zip(pivot_values) {
                *value -= factor * pivot_value;
            }
            *value_rhs -= factor * pivot_rhs;
        }
    }

    rhs.iter().all(|value| value.is_finite()).then_some(rhs)
}

/// Jacobi diagonalization for a real symmetric 3x3 matrix.
///
/// The fixed rotation count gives deterministic cost on the embedded target and
/// is ample for a 3x3 matrix. The returned eigenvectors are columns of `vectors`.
pub(super) fn symmetric_eigen_3(mut matrix: [[f32; 3]; 3]) -> Option<([f32; 3], [[f32; 3]; 3])> {
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

        // In a 3x3 matrix exactly one index is neither `p` nor `q`.
        let k = 3 - p - q;
        let akp = matrix[k][p];
        let akq = matrix[k][q];
        let new_kp = cosine * akp - sine * akq;
        let new_kq = sine * akp + cosine * akq;
        matrix[k][p] = new_kp;
        matrix[p][k] = new_kp;
        matrix[k][q] = new_kq;
        matrix[q][k] = new_kq;

        let c2 = cosine * cosine;
        let s2 = sine * sine;
        let two_sc = 2.0 * sine * cosine;
        matrix[p][p] = c2 * app - two_sc * apq + s2 * aqq;
        matrix[q][q] = s2 * app + two_sc * apq + c2 * aqq;
        matrix[p][q] = 0.0;
        matrix[q][p] = 0.0;

        for row in &mut vectors {
            let vip = row[p];
            let viq = row[q];
            row[p] = cosine * vip - sine * viq;
            row[q] = sine * vip + cosine * viq;
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

pub(super) fn matrix_vector(matrix: [[f32; 3]; 3], vector: [f32; 3]) -> [f32; 3] {
    [
        matrix[0][0] * vector[0] + matrix[0][1] * vector[1] + matrix[0][2] * vector[2],
        matrix[1][0] * vector[0] + matrix[1][1] * vector[1] + matrix[1][2] * vector[2],
        matrix[2][0] * vector[0] + matrix[2][1] * vector[1] + matrix[2][2] * vector[2],
    ]
}

pub(super) fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub(super) fn bit_count_u32(mut value: u32) -> u32 {
    let mut count = 0;
    while value != 0 {
        value &= value - 1;
        count += 1;
    }
    count
}

pub(super) fn bit_count_u8(mut value: u8) -> u32 {
    let mut count = 0;
    while value != 0 {
        value &= value - 1;
        count += 1;
    }
    count
}

pub(super) fn min_f32(a: f32, b: f32) -> f32 {
    if a < b { a } else { b }
}

pub(super) fn max_f32(a: f32, b: f32) -> f32 {
    if a > b { a } else { b }
}

pub(super) fn clamp_f32(value: f32, min: f32, max: f32) -> f32 {
    if value < min {
        min
    } else if value > max {
        max
    } else {
        value
    }
}

pub(super) fn abs_f32(value: f32) -> f32 {
    if value < 0.0 { -value } else { value }
}

pub(super) fn sqrt_approx(value: f32) -> f32 {
    super::super::sqrt::sqrt_approx(value)
}
