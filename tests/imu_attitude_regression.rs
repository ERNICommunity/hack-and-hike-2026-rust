#[path = "../src/app/ui/views/imu/attitude.rs"]
mod attitude;

use attitude::Tracker;

const ANGLE_EPSILON_DEG: f32 = 0.01;

fn wrap_degrees(mut value: f32) -> f32 {
    while value > 180.0 {
        value -= 360.0;
    }
    while value < -180.0 {
        value += 360.0;
    }
    value
}

fn angular_distance(a: f32, b: f32) -> f32 {
    wrap_degrees(a - b).abs()
}

fn assert_angle_eq(actual: f32, expected: f32) {
    assert!(
        angular_distance(actual, expected) <= ANGLE_EPSILON_DEG,
        "expected {expected} deg, got {actual} deg"
    );
}

fn positive_pole_observable(roll_deg: f32, yaw_deg: f32) -> f32 {
    wrap_degrees(yaw_deg + roll_deg)
}

fn negative_pole_observable(roll_deg: f32, yaw_deg: f32) -> f32 {
    wrap_degrees(yaw_deg - roll_deg)
}

#[test]
fn positive_physical_pitch_sweep_0_through_180_keeps_world_continuous() {
    let mut tracker = Tracker::new();

    let _ = tracker.update(1, 0.0, -90.0, 0.0);
    let _ = tracker.update(2, 90.0, -45.0, 0.0);
    let _ = tracker.update(3, 90.0, 0.0, 0.0);
    let _ = tracker.update(4, 90.0, 45.0, 0.0);
    let before_yaw = tracker.update(5, 90.0, 89.0, 0.0);
    let after_yaw = tracker.update(6, -90.0, 90.0, 0.0);

    assert_angle_eq(after_yaw, 180.0);
    assert_angle_eq(
        positive_pole_observable(90.0, before_yaw),
        positive_pole_observable(-90.0, after_yaw),
    );
}

#[test]
fn negative_physical_pitch_sweep_0_through_minus_180_keeps_world_continuous() {
    let mut tracker = Tracker::new();

    let _ = tracker.update(1, 0.0, -90.0, 0.0);
    let _ = tracker.update(2, -90.0, -45.0, 0.0);
    let _ = tracker.update(3, -90.0, 0.0, 0.0);
    let _ = tracker.update(4, -90.0, 45.0, 0.0);
    let before_yaw = tracker.update(5, -90.0, 89.0, 0.0);
    let after_yaw = tracker.update(6, 90.0, 90.0, 0.0);

    assert_angle_eq(after_yaw, 180.0);
    assert_angle_eq(
        positive_pole_observable(-90.0, before_yaw),
        positive_pole_observable(90.0, after_yaw),
    );
}

#[test]
fn negative_camera_pole_crossing_is_continuous_in_both_directions() {
    let mut forward = Tracker::new();
    let before_forward = forward.update(1, -90.0, -89.0, 0.0);
    let after_forward = forward.update(2, 90.0, -89.0, 0.0);
    assert_angle_eq(after_forward, 180.0);
    assert_angle_eq(
        negative_pole_observable(-90.0, before_forward),
        negative_pole_observable(90.0, after_forward),
    );

    let mut reverse = Tracker::new();
    let before_reverse = reverse.update(1, 90.0, -89.0, 0.0);
    let after_reverse = reverse.update(2, -90.0, -89.0, 0.0);
    assert_angle_eq(after_reverse, 180.0);
    assert_angle_eq(
        negative_pole_observable(90.0, before_reverse),
        negative_pole_observable(-90.0, after_reverse),
    );
}

#[test]
fn positive_camera_pole_crossing_is_continuous_in_both_directions() {
    let mut forward = Tracker::new();
    let before_forward = forward.update(1, 90.0, 89.0, 0.0);
    let after_forward = forward.update(2, -90.0, 89.0, 0.0);
    assert_angle_eq(after_forward, 180.0);
    assert_angle_eq(
        positive_pole_observable(90.0, before_forward),
        positive_pole_observable(-90.0, after_forward),
    );

    let mut reverse = Tracker::new();
    let before_reverse = reverse.update(1, -90.0, 89.0, 0.0);
    let after_reverse = reverse.update(2, 90.0, 89.0, 0.0);
    assert_angle_eq(after_reverse, 180.0);
    assert_angle_eq(
        positive_pole_observable(-90.0, before_reverse),
        positive_pole_observable(90.0, after_reverse),
    );
}

#[test]
fn pole_compensation_uses_y_down_screen_signs_for_non_half_turn_changes() {
    let mut negative = Tracker::new();
    let negative_before = negative.update(1, 10.0, -89.0, 20.0);
    let negative_after = negative.update(2, 40.0, -89.0, 20.0);
    assert_angle_eq(negative_after, 50.0);
    assert_angle_eq(
        negative_pole_observable(10.0, negative_before),
        negative_pole_observable(40.0, negative_after),
    );

    let mut positive = Tracker::new();
    let positive_before = positive.update(1, 10.0, 89.0, 20.0);
    let positive_after = positive.update(2, 40.0, 89.0, 20.0);
    assert_angle_eq(positive_after, -10.0);
    assert_angle_eq(
        positive_pole_observable(10.0, positive_before),
        positive_pole_observable(40.0, positive_after),
    );
}

#[test]
fn magnetic_half_turn_reacquisition_does_not_apply_the_branch_twice() {
    let mut tracker = Tracker::new();
    let _ = tracker.update(1, 90.0, 89.0, 0.0);
    let crossed_yaw = tracker.update(2, -90.0, 89.0, 0.0);

    let reacquired_yaw = tracker.update(3, -90.0, 85.0, 180.0);
    assert_angle_eq(crossed_yaw, reacquired_yaw);
}

#[test]
fn transient_magnetic_half_turn_does_not_flip_compass_branch() {
    let mut tracker = Tracker::new();
    let _ = tracker.update(1, 90.0, 89.0, 0.0);
    let crossed_yaw = tracker.update(2, -90.0, 89.0, 0.0);

    // One fused sample briefly lands on the opposite magnetic branch, then the
    // fusion estimate returns. Neither frame may visibly leave the far-side
    // compass orientation established by the pole crossing.
    let transient = tracker.update(3, -90.0, 85.0, 180.0);
    let recovered = tracker.update(4, -90.0, 82.0, 0.0);

    assert_angle_eq(transient, crossed_yaw);
    assert_angle_eq(recovered, crossed_yaw);
}

#[test]
fn sustained_magnetic_half_turn_is_committed_without_visual_jump() {
    let mut tracker = Tracker::new();
    let _ = tracker.update(1, 90.0, 89.0, 0.0);
    let crossed_yaw = tracker.update(2, -90.0, 89.0, 0.0);

    let candidate_1 = tracker.update(3, -90.0, 85.0, 180.0);
    let candidate_2 = tracker.update(4, -90.0, 82.0, 179.0);
    let candidate_3 = tracker.update(5, -90.0, 78.0, 181.0);
    let committed = tracker.update(6, -90.0, 75.0, 180.0);
    let stable = tracker.update(7, -90.0, 70.0, 180.0);

    assert_angle_eq(candidate_1, crossed_yaw);
    assert_angle_eq(candidate_2, crossed_yaw - 1.0);
    assert_angle_eq(candidate_3, crossed_yaw + 1.0);
    assert_angle_eq(committed, crossed_yaw);
    assert_angle_eq(stable, crossed_yaw);
}

#[test]
fn ordinary_roll_away_from_poles_does_not_modify_or_round_yaw() {
    let mut tracker = Tracker::new();
    let first = tracker.update(1, 10.0, 15.0, 12.375);
    let second = tracker.update(2, 75.0, 20.0, 13.625);

    assert_angle_eq(first, 12.375);
    assert_angle_eq(second, 13.625);
}

#[test]
fn leaving_and_reentering_the_imu_view_reseeds_continuity() {
    let mut tracker = Tracker::new();
    let _ = tracker.update(1, 90.0, 89.0, 0.0);
    let _ = tracker.update(2, -90.0, 89.0, 0.0);

    tracker.reset();

    let returned_yaw = tracker.update(3, 130.0, 88.0, 47.25);
    assert_angle_eq(returned_yaw, 47.25);
}

#[test]
fn long_sample_discontinuity_near_pole_reseeds_instead_of_inferring_crossing() {
    let mut tracker = Tracker::new();
    let _ = tracker.update(10, 90.0, 89.0, 0.0);

    let returned_yaw = tracker.update(29, -90.0, 89.0, 37.5);
    assert_angle_eq(returned_yaw, 37.5);
}

#[test]
fn small_replace_latest_revision_skips_remain_continuous() {
    let mut tracker = Tracker::new();
    let _ = tracker.update(100, 90.0, 89.0, 0.0);

    let yaw = tracker.update(104, -90.0, 89.0, 0.0);
    assert_angle_eq(yaw, 180.0);
}
