#![allow(
    dead_code,
    reason = "host regression harness intentionally compiles only part of the production IMU fusion module"
)]

#[derive(Clone, Copy, Debug, Default)]
struct Orientation {
    roll_deg: f32,
    pitch_deg: f32,
    yaw_deg: f32,
    gravity_screen: [f32; 3],
    north_screen: [f32; 3],
}

#[path = "../src/capabilities/imu/fusion.rs"]
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

fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn settle_heading(
    fusion: &mut Fusion,
    accel_g: [f32; 3],
    field_ut: [f32; 3],
    samples: usize,
) -> Orientation {
    let mut orientation = Orientation::default();
    for _ in 0..samples {
        orientation = fusion.update(accel_g, [0.0, 0.0, 0.0], 0.01, 0.0, Some(field_ut), 0.98);
    }
    orientation
}

#[test]
fn flat_crossing_does_not_leave_a_sticky_magnetic_branch_offset() {
    let mut fusion = Fusion::new();

    let _ = fusion.update(
        [0.0, -1.0, 0.0],
        [0.0, 0.0, 0.0],
        0.01,
        0.0,
        Some([0.0, 0.0, 50.0]),
        0.98,
    );
    let mut orientation = settle_heading(&mut fusion, [0.0, -1.0, 0.0], [0.0, 0.0, 50.0], 12);
    assert!(angular_distance(orientation.yaw_deg, 0.0) < 1.0);

    orientation = fusion.update(
        [0.0, 0.0, 1.0],
        [-100.0, 0.0, 0.0],
        0.01,
        0.0,
        Some([0.0, 50.0, 0.0]),
        0.98,
    );
    assert!(angular_distance(orientation.yaw_deg, 0.0) < 1.0);

    orientation = settle_heading(&mut fusion, [0.0, 1.0, 0.0], [0.0, 0.0, -50.0], 16);
    assert!(
        angular_distance(orientation.yaw_deg, 180.0) < 2.0,
        "expected raw far-side magnetic branch near 180 deg, got {} deg",
        orientation.yaw_deg
    );

    let _ = fusion.update(
        [0.0, 0.0, 1.0],
        [100.0, 0.0, 0.0],
        0.01,
        0.0,
        Some([0.0, 50.0, 0.0]),
        0.98,
    );
    orientation = settle_heading(&mut fusion, [0.0, -1.0, 0.0], [0.0, 0.0, 50.0], 16);
    assert!(
        angular_distance(orientation.yaw_deg, 0.0) < 2.0,
        "expected original magnetic branch near 0 deg, got {} deg",
        orientation.yaw_deg
    );
}

#[test]
fn full_basis_stays_continuous_through_camera_forward_pole() {
    let mut fusion = Fusion::new();
    let mut previous = fusion.update([0.0, -1.0, 0.0], [0.0, 0.0, 0.0], 0.01, 0.0, None, 0.98);

    // Roll the physical device from upright through display-flat to the other
    // side in two-degree increments. Euler yaw is allowed to change branch at
    // the pole; gravity/north are the actual orientation contract and must not.
    for degrees in (2..=178).step_by(2) {
        let radians = degrees as f32 * core::f32::consts::PI / 180.0;
        let accel = [0.0, -radians.cos(), radians.sin()];
        let current = fusion.update(accel, [0.0, 0.0, 0.0], 0.01, 0.0, None, 0.98);
        assert!(
            dot3(previous.gravity_screen, current.gravity_screen) > 0.998,
            "gravity basis jumped at {degrees} deg"
        );
        assert!(
            dot3(previous.north_screen, current.north_screen) > 0.998,
            "north basis jumped at {degrees} deg"
        );
        previous = current;
    }
}

#[test]
fn magnetic_reacquisition_is_smooth_not_a_single_frame_snap() {
    let mut fusion = Fusion::new();
    let _ = fusion.update(
        [0.0, -1.0, 0.0],
        [0.0, 0.0, 0.0],
        0.01,
        0.0,
        Some([0.0, 0.0, 50.0]),
        0.98,
    );
    let settled = settle_heading(&mut fusion, [0.0, -1.0, 0.0], [0.0, 0.0, 50.0], 12);
    assert!(angular_distance(settled.yaw_deg, 0.0) < 1.0);

    fusion.invalidate_absolute_heading();
    let mut orientation = settled;
    for _ in 0..12 {
        orientation = fusion.update(
            [0.0, -1.0, 0.0],
            [0.0, 0.0, 0.0],
            0.01,
            0.0,
            Some([0.0, 0.0, -50.0]),
            0.98,
        );
    }
    assert!(
        angular_distance(orientation.yaw_deg, 0.0) < 10.0,
        "reacquisition snapped too far in one confirmation window: {} deg",
        orientation.yaw_deg
    );

    for _ in 0..520 {
        orientation = fusion.update(
            [0.0, -1.0, 0.0],
            [0.0, 0.0, 0.0],
            0.01,
            0.0,
            Some([0.0, 0.0, -50.0]),
            0.98,
        );
    }
    assert!(
        angular_distance(orientation.yaw_deg, 180.0) < 5.0,
        "smooth reacquisition failed to converge: {} deg",
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

    let mut orientation = settle_heading(&mut fusion, [0.0, -1.0, 0.0], [0.0, 0.0, 50.0], 12);
    assert!(angular_distance(orientation.yaw_deg, 0.0) < 1.0);

    let mut max_deviation = 0.0f32;
    for index in 0..80 {
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
