//! Regression tests for the orientation fusion.

use hack_and_hike_core::imu::{
    Orientation,
    fusion::{Fusion, Gains},
};

/// Fusion that takes gravity directly from the accelerometer, with no
/// smoothing. So in the scenarios below, the heading depends only on the
/// magnetic correction.
fn fusion() -> Fusion {
    Fusion::with_gains(Gains {
        roll_pitch_alpha: 0.0,
        ..Gains::PRODUCTION
    })
}

/// The difference between two angles in degrees, 0 to 180.
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

/// The dot product of two vectors.
fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Run `samples` updates of a still board, 10 ms apart, with the same
/// acceleration and magnetic field (body frame). Returns the last orientation.
fn settle_heading(
    fusion: &mut Fusion,
    accel_g: [f32; 3],
    field_ut: [f32; 3],
    samples: usize,
) -> Orientation {
    let mut orientation = Orientation::default();
    for _ in 0..samples {
        orientation = fusion.update(accel_g, [0.0, 0.0, 0.0], 0.01, Some(field_ut));
    }
    orientation
}

#[test]
fn flat_crossing_does_not_leave_a_sticky_magnetic_branch_offset() {
    let mut fusion = fusion();

    // Gravity along screen +z, north along screen +x: heading 0.
    let _ = fusion.update(
        [0.0, -1.0, 0.0],
        [0.0, 0.0, 0.0],
        0.01,
        Some([0.0, 0.0, 50.0]),
    );
    let mut orientation = settle_heading(&mut fusion, [0.0, -1.0, 0.0], [0.0, 0.0, 50.0], 12);
    assert!(angular_distance(orientation.yaw_deg, 0.0) < 1.0);

    // One step with gravity along screen +x. The heading is not defined
    // there, so the last heading is kept.
    orientation = fusion.update(
        [0.0, 0.0, 1.0],
        [-100.0, 0.0, 0.0],
        0.01,
        Some([0.0, 50.0, 0.0]),
    );
    assert!(angular_distance(orientation.yaw_deg, 0.0) < 1.0);

    // Gravity along screen -z, north along screen -x: heading 180.
    orientation = settle_heading(&mut fusion, [0.0, 1.0, 0.0], [0.0, 0.0, -50.0], 16);
    assert!(
        angular_distance(orientation.yaw_deg, 180.0) < 2.0,
        "expected raw far-side magnetic branch near 180 deg, got {} deg",
        orientation.yaw_deg
    );

    // The same way back. The heading must return to 0 without an offset.
    let _ = fusion.update(
        [0.0, 0.0, 1.0],
        [100.0, 0.0, 0.0],
        0.01,
        Some([0.0, 50.0, 0.0]),
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
    let mut fusion = fusion();
    let mut previous = fusion.update([0.0, -1.0, 0.0], [0.0, 0.0, 0.0], 0.01, None);

    // Turn the board in steps of two degrees, so that gravity moves from
    // screen +z, through screen +x, to screen -z. Where gravity points along
    // screen x (the camera direction), the heading is not defined, so the
    // Euler yaw may jump. The gravity and north vectors describe the
    // orientation, so they must not jump.
    for degrees in (2..=178).step_by(2) {
        let radians = degrees as f32 * core::f32::consts::PI / 180.0;
        let accel = [0.0, -radians.cos(), radians.sin()];
        let current = fusion.update(accel, [0.0, 0.0, 0.0], 0.01, None);
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
    let mut fusion = fusion();
    let _ = fusion.update(
        [0.0, -1.0, 0.0],
        [0.0, 0.0, 0.0],
        0.01,
        Some([0.0, 0.0, 50.0]),
    );
    let settled = settle_heading(&mut fusion, [0.0, -1.0, 0.0], [0.0, 0.0, 50.0], 12);
    assert!(angular_distance(settled.yaw_deg, 0.0) < 1.0);

    // After the lock is forgotten, the field points the opposite way. North
    // must turn slowly towards it, not jump.
    fusion.invalidate_absolute_heading();
    let mut orientation = settled;
    for _ in 0..12 {
        orientation = fusion.update(
            [0.0, -1.0, 0.0],
            [0.0, 0.0, 0.0],
            0.01,
            Some([0.0, 0.0, -50.0]),
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
            Some([0.0, 0.0, -50.0]),
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
    let mut fusion = fusion();

    let _ = fusion.update(
        [0.0, -1.0, 0.0],
        [0.0, 0.0, 0.0],
        0.01,
        Some([0.0, 0.0, 50.0]),
    );

    let mut orientation = settle_heading(&mut fusion, [0.0, -1.0, 0.0], [0.0, 0.0, 50.0], 12);
    assert!(angular_distance(orientation.yaw_deg, 0.0) < 1.0);

    // The field direction alternates between about 6 degrees to the left and
    // to the right of north. The filter and the deadband must keep the
    // heading still.
    let mut max_deviation = 0.0f32;
    for index in 0..80 {
        let noisy_x = if index % 2 == 0 { 5.0 } else { -5.0 };
        orientation = fusion.update(
            [0.0, -1.0, 0.0],
            [0.0, 0.0, 0.0],
            0.01,
            Some([noisy_x, 0.0, 50.0]),
        );
        max_deviation = max_deviation.max(angular_distance(orientation.yaw_deg, 0.0));
    }

    assert!(
        max_deviation < 0.5,
        "stationary magnetic noise moved yaw by {max_deviation} deg"
    );
}
