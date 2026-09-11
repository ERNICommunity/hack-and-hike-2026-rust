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

fn settle_heading(
    fusion: &mut Fusion,
    accel_g: [f32; 3],
    field_ut: [f32; 3],
    samples: usize,
) -> Orientation {
    let mut orientation = Orientation::default();
    for _ in 0..samples {
        orientation = fusion.update(
            accel_g,
            [0.0, 0.0, 0.0],
            0.01,
            0.0,
            Some(field_ut),
            0.98,
        );
    }
    orientation
}

#[test]
fn flat_crossing_does_not_leave_a_sticky_magnetic_branch_offset() {
    let mut fusion = Fusion::new();

    // Establish absolute heading on the first side of the flat-pose singularity.
    let _ = fusion.update(
        [0.0, -1.0, 0.0],
        [0.0, 0.0, 0.0],
        0.01,
        0.0,
        Some([0.0, 0.0, 50.0]),
        0.98,
    );
    let mut orientation = settle_heading(
        &mut fusion,
        [0.0, -1.0, 0.0],
        [0.0, 0.0, 50.0],
        12,
    );
    assert!(angular_distance(orientation.yaw_deg, 0.0) < 1.0);

    // Cross the singular flat pose while moving. Fusion must discard the old
    // magnetic filter window rather than carrying it across an undefined azimuth.
    orientation = fusion.update(
        [0.0, 0.0, 1.0],
        [-100.0, 0.0, 0.0],
        0.01,
        0.0,
        Some([0.0, 50.0, 0.0]),
        0.98,
    );
    assert!(angular_distance(orientation.yaw_deg, 0.0) < 1.0);

    // The raw tilt-compensated heading is on the opposite Euler branch here.
    // Fusion may reacquire that raw branch; presentation continuity is handled
    // by attitude.rs. What matters here is that fusion does not invent and keep
    // an additional persistent 180-degree branch offset of its own.
    orientation = settle_heading(
        &mut fusion,
        [0.0, 1.0, 0.0],
        [0.0, 0.0, -50.0],
        16,
    );
    assert!(
        angular_distance(orientation.yaw_deg, 180.0) < 1.0,
        "expected raw far-side magnetic branch near 180 deg, got {} deg",
        orientation.yaw_deg
    );

    // Cross back through flat and settle on the original side. A sticky fusion
    // branch offset would leave yaw at 180; the correct raw heading returns to 0.
    let _ = fusion.update(
        [0.0, 0.0, 1.0],
        [100.0, 0.0, 0.0],
        0.01,
        0.0,
        Some([0.0, 50.0, 0.0]),
        0.98,
    );
    orientation = settle_heading(
        &mut fusion,
        [0.0, -1.0, 0.0],
        [0.0, 0.0, 50.0],
        16,
    );
    assert!(
        angular_distance(orientation.yaw_deg, 0.0) < 1.0,
        "expected original magnetic branch near 0 deg, got {} deg",
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

    let mut orientation = settle_heading(
        &mut fusion,
        [0.0, -1.0, 0.0],
        [0.0, 0.0, 50.0],
        12,
    );
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
