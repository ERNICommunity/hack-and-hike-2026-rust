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
// A large single-frame fused-yaw correction while a large pole offset is active
// is magnetic branch recovery, not physical motion: no real 100 Hz sample can
// rotate the device this far. Absorb it into the presentation offset so magnetic
// recovery cannot make the world jump. The correction need not be exactly 180
// degrees because gyro integration accumulates error while heading is undefined.
const MIN_POLE_RECOVERY_DELTA_DEG: f32 = 90.0;
const MIN_ACTIVE_POLE_OFFSET_DEG: f32 = 90.0;
const RECOVERY_BRANCH_STABILITY_DEG: f32 = 20.0;
// A single magnetic/fusion sample can briefly land on a different branch and
// then return on the next update. Apply a matching correction provisionally so
// the display stays continuous, but require several consecutive samples before
// making that correction permanent.
const REACQUISITION_CONFIRM_SAMPLES: u8 = 4;

#[derive(Clone, Copy)]
struct PendingReacquisition {
    base_yaw_deg: f32,
    provisional_offset_deg: f32,
    samples: u8,
}

pub(super) struct Tracker {
    previous_revision: Option<u32>,
    previous_roll_deg: Option<f32>,
    previous_pitch_deg: Option<f32>,
    previous_base_yaw_deg: Option<f32>,
    yaw_offset_deg: f32,
    pending_reacquisition: Option<PendingReacquisition>,
}

impl Tracker {
    pub(super) const fn new() -> Self {
        Self {
            previous_revision: None,
            previous_roll_deg: None,
            previous_pitch_deg: None,
            previous_base_yaw_deg: None,
            yaw_offset_deg: 0.0,
            pending_reacquisition: None,
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
        self.pending_reacquisition = None;
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

        self.compensate_euler_pole(roll_deg, pitch_deg);
        let presentation_offset_deg = self.presentation_offset_for_fused_yaw(base_yaw_deg);
        self.remember(sample_revision, roll_deg, pitch_deg, base_yaw_deg);

        wrap_degrees(base_yaw_deg + presentation_offset_deg)
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
        // Screen Y points downward, so the renderer's visible roll has the
        // opposite sign from a conventional mathematical camera-Z rotation.
        // Consequently the actual pole invariants are:
        //   pitch = -90 deg -> (yaw - roll)
        //   pitch = +90 deg -> (yaw + roll)
        // Move presentation yaw with the gravity-derived roll at the negative
        // pole and against it at the positive pole.
        let yaw_compensation_deg = if same_negative_pole {
            roll_delta_deg
        } else {
            -roll_delta_deg
        };
        self.yaw_offset_deg = wrap_degrees(self.yaw_offset_deg + yaw_compensation_deg);
    }

    fn presentation_offset_for_fused_yaw(&mut self, base_yaw_deg: f32) -> f32 {
        if let Some(mut pending) = self.pending_reacquisition {
            let remains_on_candidate_branch =
                abs_f32(wrap_degrees(base_yaw_deg - pending.base_yaw_deg))
                    <= RECOVERY_BRANCH_STABILITY_DEG;
            if remains_on_candidate_branch {
                pending.samples = pending.samples.saturating_add(1);
                let provisional_offset_deg = pending.provisional_offset_deg;
                if pending.samples >= REACQUISITION_CONFIRM_SAMPLES {
                    self.yaw_offset_deg = provisional_offset_deg;
                    self.pending_reacquisition = None;
                } else {
                    self.pending_reacquisition = Some(pending);
                }
                return provisional_offset_deg;
            }

            // This sample disproves the pending recovery and has already been
            // classified as belonging to the old branch. Do not feed it back
            // through large-delta detection against the transient sample.
            self.pending_reacquisition = None;
            return self.yaw_offset_deg;
        }

        let Some(previous_base_yaw_deg) = self.previous_base_yaw_deg else {
            return self.yaw_offset_deg;
        };

        let base_delta_deg = wrap_degrees(base_yaw_deg - previous_base_yaw_deg);
        let provisional_offset_deg = wrap_degrees(self.yaw_offset_deg - base_delta_deg);
        let pole_offset_active = abs_f32(self.yaw_offset_deg) >= MIN_ACTIVE_POLE_OFFSET_DEG;
        let large_fused_recovery = abs_f32(base_delta_deg) >= MIN_POLE_RECOVERY_DELTA_DEG;
        let reduces_pole_offset =
            abs_f32(provisional_offset_deg) < abs_f32(self.yaw_offset_deg);

        if pole_offset_active && large_fused_recovery && reduces_pole_offset {
            self.pending_reacquisition = Some(PendingReacquisition {
                base_yaw_deg,
                provisional_offset_deg,
                samples: 1,
            });
            return provisional_offset_deg;
        }

        self.yaw_offset_deg
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
