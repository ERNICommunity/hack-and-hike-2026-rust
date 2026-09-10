//! Stateful presentation attitude across the camera-up/down Euler poles.
//!
//! Fusion deliberately keeps magnetic yaw independent from the gravity-vector
//! roll/pitch representation. That is a good sensor boundary, but the renderer
//! cannot treat every observed Euler triple as contiguous: at displayed pitch
//! +/-90 degrees, Euler roll and yaw describe the same remaining degree of
//! freedom. The gravity-derived roll can therefore jump while the real world
//! orientation is continuous.

// Only couple roll/yaw where the Euler representation is genuinely close to its
// pole. Normal roll behavior outside this band remains exactly as before.
const POLE_LOCK_MIN_PITCH_DEG: f32 = 80.0;
// CPU1 publishes fusion at 100 Hz. Renderer-side replacement may legitimately
// skip a few revisions, but a gap larger than 80 ms is no longer treated as an
// observed pole crossing. Reseed instead of inventing motion that was not seen.
const MAX_CONTIGUOUS_REVISION_GAP: u32 = 8;
// Fusion can later reacquire the absolute magnetic-heading branch after a pole
// crossing. A genuine branch catch-up is a half-turn in both the fused yaw and
// the presentation offset; keep some tolerance for approximate embedded math.
const HALF_TURN_MATCH_TOLERANCE_DEG: f32 = 20.0;

pub(super) struct Tracker {
    previous_revision: Option<u32>,
    previous_roll_deg: Option<f32>,
    previous_pitch_deg: Option<f32>,
    previous_base_yaw_deg: Option<f32>,
    yaw_offset_deg: f32,
}

impl Tracker {
    pub(super) const fn new() -> Self {
        Self {
            previous_revision: None,
            previous_roll_deg: None,
            previous_pitch_deg: None,
            previous_base_yaw_deg: None,
            yaw_offset_deg: 0.0,
        }
    }

    /// Forget renderer-only continuity state. The IMU view calls this whenever
    /// it is entered so a hidden interval can never be interpreted as an
    /// observed pole transition.
    pub(super) fn reset(&mut self) {
        self.previous_revision = None;
        self.previous_roll_deg = None;
        self.previous_pitch_deg = None;
        self.previous_base_yaw_deg = None;
        self.yaw_offset_deg = 0.0;
    }

    /// Return the presentation yaw corresponding to one already-projected
    /// roll/pitch sample. `sample_revision` is the CPU1 publication revision.
    pub(super) fn update(
        &mut self,
        sample_revision: u32,
        roll_deg: f32,
        pitch_deg: f32,
        base_yaw_deg: f32,
    ) -> f32 {
        let base_yaw_deg = wrap_degrees(base_yaw_deg);

        if !self.is_continuous(sample_revision) {
            self.reseed(sample_revision, roll_deg, pitch_deg, base_yaw_deg);
            return base_yaw_deg;
        }

        self.absorb_fused_half_turn(base_yaw_deg);
        self.compensate_euler_pole(roll_deg, pitch_deg);
        self.remember(sample_revision, roll_deg, pitch_deg, base_yaw_deg);

        wrap_degrees(base_yaw_deg + self.yaw_offset_deg)
    }

    fn is_continuous(&self, sample_revision: u32) -> bool {
        let Some(previous_revision) = self.previous_revision else {
            return false;
        };

        // A repeated revision is a harmless redraw of the same semantic sample.
        // Wrapping subtraction also keeps u32 revision rollover continuous.
        sample_revision.wrapping_sub(previous_revision) <= MAX_CONTIGUOUS_REVISION_GAP
    }

    fn reseed(&mut self, sample_revision: u32, roll_deg: f32, pitch_deg: f32, base_yaw_deg: f32) {
        self.reset();
        self.remember(sample_revision, roll_deg, pitch_deg, base_yaw_deg);
    }

    fn remember(&mut self, sample_revision: u32, roll_deg: f32, pitch_deg: f32, base_yaw_deg: f32) {
        self.previous_revision = Some(sample_revision);
        self.previous_roll_deg = Some(roll_deg);
        self.previous_pitch_deg = Some(pitch_deg);
        self.previous_base_yaw_deg = Some(base_yaw_deg);
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

fn abs_f32(value: f32) -> f32 {
    if value < 0.0 { -value } else { value }
}
