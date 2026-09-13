//! Small helpers for `[f32; 3]` vectors used by fusion and calibration.

pub fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

pub fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

pub fn scale(v: [f32; 3], factor: f32) -> [f32; 3] {
    [v[0] * factor, v[1] * factor, v[2] * factor]
}

pub fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

pub fn norm(v: [f32; 3]) -> f32 {
    libm::sqrtf(dot(v, v))
}

/// Largest absolute component.
pub fn max_abs(v: [f32; 3]) -> f32 {
    v[0].abs().max(v[1].abs()).max(v[2].abs())
}

/// Unit vector in the direction of `v`, or `None` for a (near) zero vector.
pub fn normalize(v: [f32; 3]) -> Option<[f32; 3]> {
    const MIN_NORM_SQUARED: f32 = 1.0e-6;
    let norm_squared = dot(v, v);
    (norm_squared >= MIN_NORM_SQUARED).then(|| scale(v, 1.0 / libm::sqrtf(norm_squared)))
}

/// `a + t * (b - a)`.
pub fn lerp(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    add(a, scale(sub(b, a), t))
}

pub fn matrix_vector(matrix: [[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
    [dot(matrix[0], v), dot(matrix[1], v), dot(matrix[2], v)]
}
