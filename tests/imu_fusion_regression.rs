#[derive(Clone, Copy, Debug, Default)]
struct Orientation {
    roll_deg: f32,
    pitch_deg: f32,
    yaw_deg: f32,
}

#[path = "../src/services/imu/fusion.rs"]
mod fusion;

use fusion::Fusion;

fn angular_distance(a: f32, b: f32) -> f32 {
    let mut delta = a - b;
    while delta > 180.0 {
        delta -= 360.0;
    }
    while delta < -180.0 {
        delta += 360.0;
    }
    delta.abs()
}

#[test]
fn magnetic_heading_branch_stays_continuous_through_flat_pose() {
    let mut fusion = Fusion::new();

    // Start with the camera-forward axis horizontal and magnetic north aligned
    // with it. The first call only initializes gravity; the following samples
    // establish the absolute magnetic lock at yaw 0.
    let _ = fusion.update(
        [0.0, -1.0, 0.0],
        [0.0, 0.0, 0.0],
        0.01,
        0.0,
        Some([0.0, 0.0, 50.0]),
        0.98,
    );
    let mut orientation = Orientation::default();
    for _ in 0..10 {
        orientation = fusion.update(
            [0.0, -1.0, 0.0],
            [0.0, 0.0, 0.0],
            0.01,
            0.0,
            Some([0.0, 0.0, 50.0]),
            0.98,
        );
    }
    assert!(angular_distance(orientation.yaw_deg, 0.0) < 1.0);

    // Pitch rapidly through the flat pose. In screen coordinates gravity is now
    // +X, so camera-forward azimuth is singular. The gyro rate is perpendicular
    // to gravity: it arms magnetic recovery without changing yaw.
    orientation = fusion.update(
        [0.0, 0.0, 1.0],
        [-100.0, 0.0, 0.0],
        0.01,
        0.0,
        Some([0.0, 50.0, 0.0]),
        0.98,
    );
    assert!(angular_distance(orientation.yaw_deg, 0.0) < 1.0);

    // On the far side the raw camera-forward magnetic heading is 180 degrees,
    // even though no yaw rotation occurred. Fusion must unwrap that branch after
    // the singularity instead of performing a 180-degree magnetic recovery.
    for _ in 0..14 {
        orientation = fusion.update(
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0],
            0.01,
            0.0,
            Some([0.0, 0.0, -50.0]),
            0.98,
        );
    }

    assert!(
        angular_distance(orientation.yaw_deg, 0.0) < 1.0,
        "expected continuous yaw near 0 deg, got {} deg",
        orientation.yaw_deg
    );
}

#[test]
fn stationary_noisy_magnetic_samples_do_not_make_yaw_hunt() {
    let mut fusion = Fusion::new();

    let _ = fusion.update(
        [0.0, -1.0, 0.0],
        [0.0, 0.0, 0.0],
        0.01,
        0.0,
        Some([0.0, 0.0, 50.0]),
        0.98,
    );

    let mut orientation = Orientation::default();
    for _ in 0..12 {
        orientation = fusion.update(
            [0.0, -1.0, 0.0],
            [0.0, 0.0, 0.0],
            0.01,
            0.0,
            Some([0.0, 0.0, 50.0]),
            0.98,
        );
    }
    assert!(angular_distance(orientation.yaw_deg, 0.0) < 1.0);

    let mut max_deviation = 0.0f32;
    for index in 0..80 {
        // In this pose body X magnetic noise maps to small positive/negative
        // heading noise. Alternate it to model residual calibrated MAG jitter.
        let noisy_x = if index % 2 == 0 { 5.0 } else { -5.0 };
        orientation = fusion.update(
            [0.0, -1.0, 0.0],
            [0.0, 0.0, 0.0],
            0.01,
            0.0,
            Some([noisy_x, 0.0, 50.0]),
            0.98,
        );
        max_deviation = max_deviation.max(angular_distance(orientation.yaw_deg, 0.0));
    }

    assert!(
        max_deviation < 0.5,
        "stationary magnetic noise moved yaw by {max_deviation} deg"
    );
}