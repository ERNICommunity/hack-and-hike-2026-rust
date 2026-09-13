//! Fixed-size linear algebra used only by BMM150 calibration.

pub(super) const PARAMS: usize = 9;
const SOLVER_RELATIVE_PIVOT_EPSILON: f32 = 1.0e-6;
const JACOBI_ROTATIONS: usize = 18;
const JACOBI_CONVERGED_OFF_DIAGONAL: f32 = 1.0e-7;

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
        .fold(0.0f32, |max, value| max.max(value.abs()));
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
                if values[col].abs() > matrix[best][col].abs() {
                    row
                } else {
                    best
                }
            });
        if matrix[pivot_row][col].abs() <= pivot_epsilon {
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

/// Eigen-decomposition of a real symmetric 3x3 matrix.
pub(super) struct Eigen3 {
    pub(super) values: [f32; 3],
    /// Eigenvectors are the columns of this matrix.
    pub(super) vectors: [[f32; 3]; 3],
}

/// Jacobi diagonalization. The fixed rotation count gives deterministic cost
/// on the embedded target and is ample for a 3x3 matrix.
pub(super) fn symmetric_eigen_3(mut matrix: [[f32; 3]; 3]) -> Option<Eigen3> {
    let mut vectors = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

    for _ in 0..JACOBI_ROTATIONS {
        let (p, q) = largest_off_diagonal(matrix);
        let apq = matrix[p][q];
        if apq.abs() <= JACOBI_CONVERGED_OFF_DIAGONAL {
            break;
        }
        let app = matrix[p][p];
        let aqq = matrix[q][q];

        let tau = (aqq - app) / (2.0 * apq);
        let t = libm::copysignf(1.0, tau) / (tau.abs() + libm::sqrtf(1.0 + tau * tau));
        let cosine = 1.0 / libm::sqrtf(1.0 + t * t);
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

    let values = [matrix[0][0], matrix[1][1], matrix[2][2]];
    values
        .iter()
        .all(|value| value.is_finite())
        .then_some(Eigen3 { values, vectors })
}

/// Row and column of the largest off-diagonal element (row < column).
fn largest_off_diagonal(matrix: [[f32; 3]; 3]) -> (usize, usize) {
    let a01 = matrix[0][1].abs();
    let a02 = matrix[0][2].abs();
    let a12 = matrix[1][2].abs();
    if a01 >= a02 && a01 >= a12 {
        (0, 1)
    } else if a02 >= a12 {
        (0, 2)
    } else {
        (1, 2)
    }
}
