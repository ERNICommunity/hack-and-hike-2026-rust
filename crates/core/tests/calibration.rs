//! Magnetometer calibration on a synthetic distorted field.

use hack_and_hike_core::imu::{bmm150::Calibration, vec3};

/// Hard-iron offset of the synthetic enclosure.
const CENTER_UT: [f32; 3] = [120.0, -80.0, 40.0];
/// Soft-iron stretch factor along each axis.
const SCALES: [f32; 3] = [1.0, 0.8, 1.25];
/// Strength of the synthetic Earth field, in uT.
const EARTH_FIELD_UT: f32 = 45.0;
/// The calibration scales corrected fields to this strength, in uT.
const CALIBRATED_RADIUS_UT: f32 = 50.0;

/// What the magnetometer reports inside the synthetic enclosure when the
/// Earth's field points in the unit `direction`.
fn raw_field(direction: [f32; 3]) -> [f32; 3] {
    let mut field = CENTER_UT;
    for axis in 0..3 {
        field[axis] += SCALES[axis] * EARTH_FIELD_UT * direction[axis];
    }
    field
}

/// `count` evenly spread unit vectors. They follow a spiral from the top of
/// the sphere to the bottom, similar to a person who turns the board.
fn directions(count: usize) -> Vec<[f32; 3]> {
    let golden_angle = std::f32::consts::PI * (3.0 - 5.0f32.sqrt());
    (0..count)
        .map(|index| {
            let z = 1.0 - 2.0 * (index as f32 + 0.5) / count as f32;
            let radius = (1.0 - z * z).sqrt();
            let angle = golden_angle * index as f32;
            [radius * angle.cos(), radius * angle.sin(), z]
        })
        .collect()
}

#[test]
fn learns_hard_and_soft_iron_from_good_coverage() {
    let mut calibration = Calibration::new();
    let directions = directions(600);

    for _pass in 0..4 {
        for direction in &directions {
            calibration.observe(raw_field(*direction));
        }
        if calibration.is_ready() {
            break;
        }
    }
    assert!(
        calibration.is_ready(),
        "calibration did not converge, progress {}%",
        calibration.progress_percent()
    );
    assert_eq!(calibration.progress_percent(), 100);

    for direction in directions.iter().step_by(7) {
        let strength = vec3::norm(calibration.apply(raw_field(*direction)));
        assert!(
            (strength - CALIBRATED_RADIUS_UT).abs() < 0.05 * CALIBRATED_RADIUS_UT,
            "corrected field {strength} uT is not close to {CALIBRATED_RADIUS_UT} uT"
        );
    }
}

#[test]
fn recovers_from_a_single_disturbed_sample() {
    let mut calibration = Calibration::new();
    let directions = directions(600);

    // A magnet passes by once. The sample is learnable, but far from the real
    // center. It stretches the extrema, so their midpoint is wrong and the
    // direction bins are unbalanced. The calibration can only succeed after
    // the stalled epoch starts over.
    for direction in &directions[..50] {
        calibration.observe(raw_field(*direction));
    }
    calibration.observe([400.0, -80.0, 40.0]);

    for _pass in 0..10 {
        for direction in &directions {
            calibration.observe(raw_field(*direction));
        }
        if calibration.is_ready() {
            break;
        }
    }
    assert!(
        calibration.is_ready(),
        "calibration did not recover, progress {}%",
        calibration.progress_percent()
    );
}

#[test]
fn progress_starts_at_zero_and_ignores_implausible_samples() {
    let mut calibration = Calibration::new();
    assert_eq!(calibration.progress_percent(), 0);
    assert!(!calibration.is_ready());

    assert!(!Calibration::is_learnable([0.0, 0.0, 0.0]));
    calibration.observe([0.0, 0.0, 0.0]);
    assert_eq!(calibration.progress_percent(), 0);

    for direction in directions(40) {
        calibration.observe(raw_field(direction));
    }
    let progress = calibration.progress_percent();
    assert!(
        (1..100).contains(&progress),
        "expected partial progress, got {progress}%"
    );
    assert!(!calibration.is_ready());
}

#[test]
fn uncalibrated_fields_pass_through_unchanged() {
    let calibration = Calibration::new();
    let field = raw_field([0.0, 0.0, 1.0]);
    assert_eq!(calibration.apply(field), field);
    assert!(Calibration::is_earth_field(CALIBRATED_RADIUS_UT));
    assert!(!Calibration::is_earth_field(500.0));
}
