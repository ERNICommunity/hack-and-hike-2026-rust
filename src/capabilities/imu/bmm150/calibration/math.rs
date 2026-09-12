//! Fixed-size numerical helpers used only by BMM150 calibration.

pub(super) const PARAMS: usize = 9;
const AUGMENTED: usize = PARAMS + 1;
const SOLVER_RELATIVE_PIVOT_EPSILON: f32 = 1.0e-6;
const JACOBI_ROTATIONS: usize = 18;

pub(super) fn solve_linear_9(mut augmented: [[f32; AUGMENTED]; PARAMS]) -> Option<[f32; PARAMS]> {
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

pub(super) fn solve_linear_3(matrix: [[f32; 3]; 3], rhs: [f32; 3]) -> Option<[f32; 3]> {
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
