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
    for _ in 0..8 {
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
    for _ in 0..12 {
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