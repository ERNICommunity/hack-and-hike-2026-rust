#[path = "../src/capabilities/imu/bmm150/sqrt.rs"]
mod sqrt;

fn assert_relative(actual: f32, expected: f32, tolerance: f32) {
    let scale = expected.abs().max(f32::MIN_POSITIVE);
    let relative = (actual - expected).abs() / scale;
    assert!(
        relative <= tolerance,
        "actual={actual} expected={expected} relative_error={relative} tolerance={tolerance}"
    );
}

#[test]
fn sqrt_is_scale_stable_for_magnetic_field_norms() {
    // The previous fixed-iteration Newton solver started at `value` itself.
    // For a squared norm around 640,000 it returned roughly 10,000 after six
    // iterations instead of the correct 800, which made valid BMM150 samples
    // look like >10,000 uT disturbances.
    for (value, expected) in [
        (2.0, core::f32::consts::SQRT_2),
        (2_500.0, 50.0),
        (640_000.0, 800.0),
        (1_000_000.0, 1_000.0),
        (117_000_000.0, 10_816.654),
    ] {
        assert_relative(sqrt::sqrt_approx(value), expected, 1.0e-5);
    }
}

#[test]
fn sqrt_handles_zero_subnormals_and_infinity() {
    assert_eq!(sqrt::sqrt_approx(0.0), 0.0);
    assert_eq!(sqrt::sqrt_approx(-1.0), 0.0);

    let smallest = f32::from_bits(1);
    assert_relative(sqrt::sqrt_approx(smallest), 3.743_392e-23, 1.0e-5);
    assert_eq!(sqrt::sqrt_approx(f32::INFINITY), f32::INFINITY);
}
