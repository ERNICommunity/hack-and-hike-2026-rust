//! Stateful presentation attitude across the camera-up/down Euler poles.
//!
//! Fusion deliberately keeps magnetic yaw independent from the gravity-vector
//! roll/pitch representation. That is a good sensor boundary, but the renderer
//! cannot convert those values to a fresh Euler triple on every frame: at
//! displayed pitch +/-90 degrees, Euler roll and yaw describe the same remaining
//! degree of freedom. The gravity-derived roll can therefore jump while the real
//! world orientation is continuous.

use crate::app::model::ImuDisplay;

use super::projection::{DisplayAttitude, display_attitude};

// Only couple roll/yaw where the Euler representation is genuinely close to its
// pole. Normal roll behavior outside this band remains exactly as before.
const POLE_LOCK_MIN_PITCH_DEG: f32 = 80.0;
// Fusion can later reacquire the absolute magnetic-heading branch after a pole
// crossing. A genuine branch catch-up is a half-turn in both the fused yaw and
// the presentation offset; keep some tolerance for the approximate trig/rounding
// used by the embedded renderer.
const HALF_TURN_MATCH_TOLERANCE_DEG: f32 = 20.0;

pub(super) struct Tracker {
    previous_roll_deg: Option<f32>,
    previous_pitch_deg: Option<f32>,
    previous_base_yaw_deg: Option<f32>,
    yaw_offset_deg: f32,
}

impl Tracker {
    pub(super) const fn new() -> Self {
        Self {
            previous_roll_deg: None,
            previous_pitch_deg: None,
            previous_base_yaw_deg: None,
            yaw_offset_deg: 0.0,
        }
    }

    pub(super) fn update(&mut self, imu: &ImuDisplay) -> DisplayAttitude {
        let mut attitude = display_attitude(imu);
        let base_yaw_deg = attitude.yaw_deg as f32;

        self.absorb_fused_half_turn(base_yaw_deg);
        self.compensate_euler_pole(attitude.roll_deg, attitude.pitch_deg);

        self.previous_roll_deg = Some(attitude.roll_deg);
        self.previous_pitch_deg = Some(attitude.pitch_deg);
        self.previous_base_yaw_deg = Some(base_yaw_deg);
        attitude.yaw_deg = round_f32(wrap_degrees(base_yaw_deg + self.yaw_offset_deg));
        attitude
    }

    fn compensate_euler_pole(&mut self, roll_deg: f32, pitch_deg: f32) {
        let (Some(previous_roll_deg), Some(previous_pitch_deg)) =
            (self.previous_roll_deg, self.previous_pitch_deg)
        else {
            return;
        };

        let same_negative_pole =
            pitch_deg <= -POLE_LOCK_MIN_PITCH_DEG && previous_pitch_deg <= -POLE_LOCK_MIN_PITCH_DEG;
        let same_positive_pole =
            pitch_deg >= POLE_LOCK_MIN_PITCH_DEG && previous_pitch_deg >= POLE_LOCK_MIN_PITCH_DEG;
        if !same_negative_pole && !same_positive_pole {
            return;
        }

        let roll_delta_deg = wrap_degrees(roll_deg - previous_roll_deg);
        // For the renderer's Rz(roll) * Rx(pitch) * Ry(yaw) convention:
        //   pitch = -90 deg -> only (roll + yaw) is observable
        //   pitch = +90 deg -> only (roll - yaw) is observable
        // Counter-rotate yaw by the gravity-derived roll change so a noisy or
        // branch-flipping Euler roll cannot spin the world at either pole.
        let yaw_compensation_deg = if same_negative_pole {
            -roll_delta_deg
        } else {
            roll_delta_deg
        };
        self.yaw_offset_deg = wrap_degrees(self.yaw_offset_deg + yaw_compensation_deg);
    }

    fn absorb_fused_half_turn(&mut self, base_yaw_deg: f32) {
        let Some(previous_base_yaw_deg) = self.previous_base_yaw_deg else {
            return;
        };

        // While the forward axis is vertical, fusion intentionally bridges the
        // undefined magnetic heading with gyro yaw. After crossing the pole, a
        // later magnetic reacquisition can move that fused heading onto the new
        // 180-degree branch. If it exactly matches the half-turn already carried
        // by this presentation tracker, drop the temporary offset instead of
        // applying the branch change twice.
        let base_delta_deg = wrap_degrees(base_yaw_deg - previous_base_yaw_deg);
        let offset_is_half_turn =
            abs_f32(abs_f32(self.yaw_offset_deg) - 180.0) <= HALF_TURN_MATCH_TOLERANCE_DEG;
        let base_delta_is_half_turn =
            abs_f32(abs_f32(base_delta_deg) - 180.0) <= HALF_TURN_MATCH_TOLERANCE_DEG;
        let branches_cancel = abs_f32(wrap_degrees(self.yaw_offset_deg + base_delta_deg))
            <= HALF_TURN_MATCH_TOLERANCE_DEG;

        if offset_is_half_turn && base_delta_is_half_turn && branches_cancel {
            self.yaw_offset_deg = 0.0;
        }
    }
}

fn wrap_degrees(mut value: f32) -> f32 {
    while value > 180.0 {
        value -= 360.0;
    }
    while value < -180.0 {
        value += 360.0;
    }
    value
}

fn round_f32(value: f32) -> i32 {
    if value >= 0.0 {
        (value + 0.5) as i32
    } else {
        (value - 0.5) as i32
    }
}

fn abs_f32(value: f32) -> f32 {
    if value < 0.0 { -value } else { value }
}
