//! Scale-stable square root for BMM150 compensation and calibration math.
//!
//! Xtensa `no_std` builds do not provide the usual floating-point `sqrt`
//! method. Newton iteration is fine here, but its initial estimate must scale
//! with the input. Starting at `value` makes convergence logarithmic in the
//! magnitude and badly overestimates large magnetic norms after a fixed number
//! of iterations.

const SQRT_SEED_BIAS: u32 = 0x1fc0_0000;
const SUBNORMAL_SCALE: f32 = 16_777_216.0; // 2^24
const SUBNORMAL_SQRT_SCALE: f32 = 4096.0; // sqrt(2^24)

pub(super) fn sqrt_approx(value: f32) -> f32 {
    if value <= 0.0 {
        return 0.0;
    }
    if !value.is_finite() {
        return value;
    }

    // The bit-level seed halves the IEEE-754 exponent, giving Newton-Raphson a
    // scale-appropriate starting point. Normalize subnormals first so their
    // non-biased exponent representation does not break that seed.
    let (normalized, result_scale) = if value < f32::MIN_POSITIVE {
        (value * SUBNORMAL_SCALE, 1.0 / SUBNORMAL_SQRT_SCALE)
    } else {
        (value, 1.0)
    };
    let mut estimate = f32::from_bits((normalized.to_bits() >> 1) + SQRT_SEED_BIAS);

    // Four iterations from this seed converge to f32 precision across the
    // finite positive range, including the large squared norms seen before
    // hard-iron calibration.
    for _ in 0..4 {
        estimate = 0.5 * (estimate + normalized / estimate);
    }

    estimate * result_scale
}
