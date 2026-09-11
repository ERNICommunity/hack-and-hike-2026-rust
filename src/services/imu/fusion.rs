//! Pure gyro-bias estimation and orientation fusion.
//!
//! This module deliberately has no I2C, Embassy, or sensor-register knowledge.

use super::Orientation;

// Magnetic yaw is a long-term absolute reference. Keep its per-frame authority
// deliberately small so residual hard/soft-iron and tilt errors cannot make the
// compass hunt while the gyro already provides a smooth short-term heading.
const MAX_MAG_YAW_CORRECTION_PER_SAMPLE_DEG: f32 = 0.08;
const MAG_YAW_DEADBAND_DEG: f32 = 1.5;
const MAG_HEADING_FILTER_ALPHA: f32 = 0.18;
// The 30 Hz BMM150 is delayed relative to the gyro. Only fuse it once hand motion
// is genuinely slow; faster motion is carried by gyro yaw and corrected later.
const MAG_FUSION_MAX_RATE_DPS: f32 = 20.0;
// Require several fresh low-motion MAG frames before using the compass after a
// turn. At 30 Hz this is roughly 170 ms, enough for the AUX pipeline to settle.
const MAG_QUIET_SAMPLES_BEFORE_FUSION: u8 = 5;
// Initial/reacquired north must be consistent over multiple independent BMM150
// frames. Compare heading-minus-gyro offsets so small residual motion cancels.
const MAG_INITIAL_LOCK_SAMPLES: u8 = 5;
const MAX_MAG_INITIAL_OFFSET_JITTER_DEG: f32 = 6.0;
// A large but stable discrepancy after real motion is evidence that gyro
// integration lost angle. Reacquire only after a longer consistency proof; a
// stationary magnetic disturbance with no preceding motion remains rejected.
const MAG_RECOVERY_MIN_INNOVATION_DEG: f32 = 30.0;
const MAG_RECOVERY_SAMPLES: u8 = 8;
const MAX_MAG_RECOVERY_OFFSET_JITTER_DEG: f32 = 6.0;
// Reject poorly conditioned tilt compensation. The horizontal geomagnetic field
// should be comfortably above sensor noise, and the camera-forward heading axis
// must be at least 50% horizontal. Near its vertical singularity gyro yaw is a
// substantially better short-term heading reference than amplified MAG noise.
const MIN_MAG_HORIZONTAL_FIELD_UT: f32 = 8.0;
const MIN_HEADING_AXIS_HORIZONTAL_SQ: f32 = 0.25;
const HALF_TURN_DEG: f32 = 180.0;

#[derive(Clone, Copy)]
pub(super) struct GyroBias {
    bias_dps: [f32; 3],
    stationary_samples: u16,
    ready: bool,
}

impl GyroBias {
    pub(super) const fn new() -> Self {
        Self {
            bias_dps: [0.0; 3],
            stationary_samples: 0,
            ready: false,
        }
    }

    pub(super) fn correct(&mut self, accel_g: [f32; 3], gyro_dps: [f32; 3]) -> [f32; 3] {
        let accel_norm_sq =
            accel_g[0] * accel_g[0] + accel_g[1] * accel_g[1] + accel_g[2] * accel_g[2];
        let gyro_max = max_abs3(gyro_dps);
        let stationary = (0.90 * 0.90..=1.10 * 1.10).contains(&accel_norm_sq) && gyro_max < 3.0;

        if stationary {
            self.stationary_samples = self.stationary_samples.saturating_add(1);
            let learn = if self.ready { 0.002 } else { 0.02 };
            for axis in 0..3 {
                self.bias_dps[axis] += (gyro_dps[axis] - self.bias_dps[axis]) * learn;
            }
            if self.stationary_samples >= 100 {
                self.ready = true;
            }
        } else {
            self.stationary_samples = 0;
        }

        [
            gyro_dps[0] - self.bias_dps[0],
            gyro_dps[1] - self.bias_dps[1],
            gyro_dps[2] - self.bias_dps[2],
        ]
    }
}

#[derive(Clone, Copy)]
pub(super) struct Fusion {
    orientation: Orientation,
    gravity_body: [f32; 3],
    previous_yaw_rate_dps: Option<f32>,
    magnetic_heading_locked: bool,
    magnetic_heading_branch_offset_deg: f32,
    filtered_magnetic_heading_deg: Option<f32>,
    reselect_magnetic_heading_branch: bool,
    quiet_mag_samples: u8,
    recovery_armed: bool,
    pending_mag_offset: Option<f32>,
    pending_mag_samples: u8,
    recovery_mag_offset: Option<f32>,
    recovery_mag_samples: u8,
    initialized: bool,
}

impl Fusion {
    pub(super) const fn new() -> Self {
        Self {
            orientation: Orientation {
                roll_deg: 0.0,
                pitch_deg: 0.0,
                yaw_deg: 0.0,
            },
            gravity_body: [0.0, 0.0, 1.0],
            previous_yaw_rate_dps: None,
            magnetic_heading_locked: false,
            magnetic_heading_branch_offset_deg: 0.0,
            filtered_magnetic_heading_deg: None,
            reselect_magnetic_heading_branch: false,
            quiet_mag_samples: 0,
            recovery_armed: false,
            pending_mag_offset: None,
            pending_mag_samples: 0,
            recovery_mag_offset: None,
            recovery_mag_samples: 0,
            initialized: false,
        }
    }

    pub(super) fn invalidate_absolute_heading(&mut self) {
        self.magnetic_heading_locked = false;
        self.magnetic_heading_branch_offset_deg = 0.0;
        self.filtered_magnetic_heading_deg = None;
        self.reselect_magnetic_heading_branch = false;
        self.quiet_mag_samples = 0;
        self.recovery_armed = true;
        self.clear_pending_magnetic_candidate();
        self.clear_recovery_candidate();
    }

    pub(super) fn reset_rate_history(&mut self) {
        self.previous_yaw_rate_dps = None;
    }

    fn clear_pending_magnetic_candidate(&mut self) {
        self.pending_mag_offset = None;
        self.pending_mag_samples = 0;
    }

    fn clear_recovery_candidate(&mut self) {
        self.recovery_mag_offset = None;
        self.recovery_mag_samples = 0;
    }

    fn note_motion(&mut self) {
        self.quiet_mag_samples = 0;
        self.filtered_magnetic_heading_deg = None;
        self.recovery_armed = true;
        self.clear_pending_magnetic_candidate();
        self.clear_recovery_candidate();
    }

    pub(super) fn update(
        &mut self,
        accel_g: [f32; 3],
        gyro_dps: [f32; 3],
        dt_seconds: f32,
        roll_pitch_alpha: f32,
        magnetic_field_ut: Option<[f32; 3]>,
        yaw_alpha: f32,
    ) -> Orientation {
        let measured_gravity = normalize3(accel_g);

        if !self.initialized {
            if let Some(gravity) = measured_gravity {
                self.gravity_body = gravity;
            }
            let (roll, pitch) = attitude_from_gravity(self.gravity_body);
            self.orientation.roll_deg = roll;
            self.orientation.pitch_deg = pitch;
            self.orientation.yaw_deg = 0.0;
            self.initialized = true;
            return self.orientation;
        }

        // Propagate one gravity vector with the complete body-rate vector instead
        // of integrating roll/pitch as independent Euler angles. A yaw rotation
        // around gravity therefore leaves tilt unchanged by construction.
        let predicted_gravity = integrate_gravity(self.gravity_body, gyro_dps, dt_seconds);
        let accel_norm_sq = dot3(accel_g, accel_g);
        let accel_plausible = (0.75 * 0.75..=1.25 * 1.25).contains(&accel_norm_sq);
        let alpha = clamp_f32(roll_pitch_alpha, 0.0, 1.0);
        self.gravity_body = if accel_plausible {
            if let Some(measured) = measured_gravity {
                let blended = [
                    alpha * predicted_gravity[0] + (1.0 - alpha) * measured[0],
                    alpha * predicted_gravity[1] + (1.0 - alpha) * measured[1],
                    alpha * predicted_gravity[2] + (1.0 - alpha) * measured[2],
                ];
                normalize3(blended).unwrap_or(predicted_gravity)
            } else {
                predicted_gravity
            }
        } else {
            predicted_gravity
        };

        let (roll, pitch) = attitude_from_gravity(self.gravity_body);
        self.orientation.roll_deg = roll;
        self.orientation.pitch_deg = pitch;

        let screen_gravity =
            normalize3(screen_vector_from_body(self.gravity_body)).unwrap_or([0.0, 0.0, 1.0]);
        let screen_gyro = screen_vector_from_body(gyro_dps);

        // Yaw rate is the component of angular velocity around local gravity.
        let yaw_rate_dps = dot3(screen_gyro, screen_gravity);
        // Trapezoidal integration preserves substantially more turn angle during
        // fast acceleration/deceleration than integrating only the newest rate.
        let integrated_yaw_rate = self
            .previous_yaw_rate_dps
            .map(|previous| 0.5 * (previous + yaw_rate_dps))
            .unwrap_or(yaw_rate_dps);
        self.previous_yaw_rate_dps = Some(yaw_rate_dps);
        let predicted_yaw =
            wrap_degrees(self.orientation.yaw_deg + integrated_yaw_rate * dt_seconds);

        // The renderer's vertical-looking poses correspond to the magnetic
        // heading axis itself becoming vertical. Raw tilt-compensated heading
        // changes by 180° when that projected axis emerges on the opposite side.
        // Remember that the singularity was traversed and choose the magnetic
        // branch closest to gyro yaw once heading is observable again.
        let heading_axis_horizontal_sq = 1.0 - screen_gravity[0] * screen_gravity[0];
        if self.magnetic_heading_locked
            && heading_axis_horizontal_sq < MIN_HEADING_AXIS_HORIZONTAL_SQ
        {
            self.reselect_magnetic_heading_branch = true;
            self.filtered_magnetic_heading_deg = None;
        }

        let total_rate_dps = max_abs3(gyro_dps);
        if total_rate_dps > MAG_FUSION_MAX_RATE_DPS {
            // Never mix delayed 30 Hz magnetic observations into active motion.
            self.note_motion();
            self.orientation.yaw_deg = predicted_yaw;
            return self.orientation;
        }

        let magnetic_heading = magnetic_field_ut
            .map(screen_vector_from_body)
            .and_then(|field| gravity_compensated_heading(field, screen_gravity))
            .map(|heading| self.select_magnetic_heading_branch(predicted_yaw, heading))
            .map(|heading| self.filter_magnetic_heading(heading));

        self.orientation.yaw_deg = if let Some(heading) = magnetic_heading {
            self.fuse_magnetic_yaw(predicted_yaw, heading, yaw_alpha)
        } else {
            predicted_yaw
        };

        self.orientation
    }

    fn select_magnetic_heading_branch(&mut self, predicted_yaw: f32, heading: f32) -> f32 {
        if !self.magnetic_heading_locked {
            return heading;
        }

        if self.reselect_magnetic_heading_branch {
            let current = wrap_degrees(heading + self.magnetic_heading_branch_offset_deg);
            let alternate = wrap_degrees(current + HALF_TURN_DEG);
            let current_error = abs_f32(wrap_degrees(current - predicted_yaw));
            let alternate_error = abs_f32(wrap_degrees(alternate - predicted_yaw));

            if alternate_error < current_error {
                self.magnetic_heading_branch_offset_deg =
                    wrap_degrees(self.magnetic_heading_branch_offset_deg + HALF_TURN_DEG);
            }
            self.reselect_magnetic_heading_branch = false;
        }

        wrap_degrees(heading + self.magnetic_heading_branch_offset_deg)
    }

    fn filter_magnetic_heading(&mut self, heading: f32) -> f32 {
        let filtered = self
            .filtered_magnetic_heading_deg
            .map(|previous| {
                wrap_degrees(
                    previous + MAG_HEADING_FILTER_ALPHA * wrap_degrees(heading - previous),
                )
            })
            .unwrap_or(heading);
        self.filtered_magnetic_heading_deg = Some(filtered);
        filtered
    }

    fn fuse_magnetic_yaw(&mut self, predicted_yaw: f32, heading: f32, yaw_alpha: f32) -> f32 {
        self.quiet_mag_samples = self.quiet_mag_samples.saturating_add(1);
        if self.quiet_mag_samples < MAG_QUIET_SAMPLES_BEFORE_FUSION {
            return predicted_yaw;
        }

        let offset = wrap_degrees(heading - predicted_yaw);

        if !self.magnetic_heading_locked {
            let consistent = self
                .pending_mag_offset
                .map(|previous| {
                    abs_f32(wrap_degrees(offset - previous)) <= MAX_MAG_INITIAL_OFFSET_JITTER_DEG
                })
                .unwrap_or(false);

            if consistent {
                self.pending_mag_samples = self.pending_mag_samples.saturating_add(1);
                let previous = self.pending_mag_offset.unwrap_or(offset);
                self.pending_mag_offset = Some(wrap_degrees(
                    previous + 0.25 * wrap_degrees(offset - previous),
                ));
            } else {
                self.pending_mag_offset = Some(offset);
                self.pending_mag_samples = 1;
            }

            if self.pending_mag_samples >= MAG_INITIAL_LOCK_SAMPLES {
                let acquired_offset = self.pending_mag_offset.unwrap_or(offset);
                self.magnetic_heading_locked = true;
                self.recovery_armed = false;
                self.clear_pending_magnetic_candidate();
                self.clear_recovery_candidate();
                return wrap_degrees(predicted_yaw + acquired_offset);
            }

            return predicted_yaw;
        }

        // Large recovery is only legal after actual motion (or explicit timing/
        // saturation invalidation). Once MAG and gyro agree after a turn, disarm
        // it so a later stationary magnetic disturbance cannot redefine north.
        if abs_f32(offset) >= MAG_RECOVERY_MIN_INNOVATION_DEG {
            if !self.recovery_armed {
                self.clear_recovery_candidate();
                return predicted_yaw;
            }

            let consistent = self
                .recovery_mag_offset
                .map(|previous| {
                    abs_f32(wrap_degrees(offset - previous)) <= MAX_MAG_RECOVERY_OFFSET_JITTER_DEG
                })
                .unwrap_or(false);

            if consistent {
                self.recovery_mag_samples = self.recovery_mag_samples.saturating_add(1);
                let previous = self.recovery_mag_offset.unwrap_or(offset);
                self.recovery_mag_offset = Some(wrap_degrees(
                    previous + 0.25 * wrap_degrees(offset - previous),
                ));
            } else {
                self.recovery_mag_offset = Some(offset);
                self.recovery_mag_samples = 1;
            }

            if self.recovery_mag_samples >= MAG_RECOVERY_SAMPLES {
                let recovered_offset = self.recovery_mag_offset.unwrap_or(offset);
                self.recovery_armed = false;
                self.clear_recovery_candidate();
                return wrap_degrees(predicted_yaw + recovered_offset);
            }

            return predicted_yaw;
        }

        self.recovery_armed = false;
        self.clear_recovery_candidate();
        if abs_f32(offset) <= MAG_YAW_DEADBAND_DEG {
            return predicted_yaw;
        }

        let requested = (1.0 - clamp_f32(yaw_alpha, 0.0, 1.0)) * offset;
        let applied = clamp_f32(
            requested,
            -MAX_MAG_YAW_CORRECTION_PER_SAMPLE_DEG,
            MAX_MAG_YAW_CORRECTION_PER_SAMPLE_DEG,
        );
        wrap_degrees(predicted_yaw + applied)
    }
}

const PI: f32 = 3.14159265358979323846;
const RAD_TO_DEG: f32 = 180.0 / PI;
const DEG_TO_RAD: f32 = PI / 180.0;

fn radians_to_degrees(value: f32) -> f32 {
    value * RAD_TO_DEG
}

fn clamp_f32(value: f32, min: f32, max: f32) -> f32 {
    if value < min {
        min
    } else if value > max {
        max
    } else {
        value
    }
}

pub(super) fn max_abs3(value: [f32; 3]) -> f32 {
    let a = abs_f32(value[0]);
    let b = abs_f32(value[1]);
    let c = abs_f32(value[2]);
    if a > b {
        if a > c { a } else { c }
    } else if b > c {
        b
    } else {
        c
    }
}

fn abs_f32(value: f32) -> f32 {
    if value < 0.0 { -value } else { value }
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

fn sqrt_approx(value: f32) -> f32 {
    if value <= 0.0 {
        return 0.0;
    }

    let mut estimate = if value > 1.0 { value } else { 1.0 };
    for _ in 0..6 {
        estimate = 0.5 * (estimate + value / estimate);
    }
    estimate
}

fn screen_vector_from_body(value: [f32; 3]) -> [f32; 3] {
    [value[2], -value[0], -value[1]]
}

fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalize3(value: [f32; 3]) -> Option<[f32; 3]> {
    let norm_sq = dot3(value, value);
    if norm_sq < 0.01 {
        return None;
    }
    let inverse = 1.0 / sqrt_approx(norm_sq);
    Some([value[0] * inverse, value[1] * inverse, value[2] * inverse])
}

fn integrate_gravity(gravity: [f32; 3], gyro_dps: [f32; 3], dt_seconds: f32) -> [f32; 3] {
    let omega = [
        gyro_dps[0] * DEG_TO_RAD,
        gyro_dps[1] * DEG_TO_RAD,
        gyro_dps[2] * DEG_TO_RAD,
    ];
    // Coordinates of an inertially fixed gravity vector in a rotating body obey
    // g_dot = -omega x g = g x omega.
    let derivative = cross3(gravity, omega);
    let predicted = [
        gravity[0] + derivative[0] * dt_seconds,
        gravity[1] + derivative[1] * dt_seconds,
        gravity[2] + derivative[2] * dt_seconds,
    ];
    normalize3(predicted).unwrap_or(gravity)
}

fn attitude_from_gravity(gravity: [f32; 3]) -> (f32, f32) {
    let [gx, gy, gz] = gravity;
    let roll = radians_to_degrees(atan2_approx(gy, gz));
    let pitch = radians_to_degrees(atan2_approx(-gx, sqrt_approx(gy * gy + gz * gz)));
    (roll, pitch)
}

/// Compute magnetic heading without inventing a leveling rotation.
///
/// Project both magnetic north and the fixed camera-forward (+X) heading axis
/// onto the plane perpendicular to gravity, then measure their signed angle
/// around gravity. This is tilt-invariant wherever camera-forward azimuth is
/// physically defined. Fusion explicitly unwraps the branch across its vertical
/// singularity so the far side cannot trigger a 180-degree magnetic recovery.
fn gravity_compensated_heading(field: [f32; 3], gravity: [f32; 3]) -> Option<f32> {
    let field_along_gravity = dot3(field, gravity);
    let horizontal_field = [
        field[0] - gravity[0] * field_along_gravity,
        field[1] - gravity[1] * field_along_gravity,
        field[2] - gravity[2] * field_along_gravity,
    ];
    let horizontal_field_sq = dot3(horizontal_field, horizontal_field);
    if horizontal_field_sq < MIN_MAG_HORIZONTAL_FIELD_UT * MIN_MAG_HORIZONTAL_FIELD_UT {
        return None;
    }

    let forward_along_gravity = gravity[0];
    let horizontal_forward = [
        1.0 - gravity[0] * forward_along_gravity,
        -gravity[1] * forward_along_gravity,
        -gravity[2] * forward_along_gravity,
    ];
    if dot3(horizontal_forward, horizontal_forward) < MIN_HEADING_AXIS_HORIZONTAL_SQ {
        return None;
    }

    let sine = -dot3(gravity, cross3(horizontal_forward, horizontal_field));
    let cosine = dot3(horizontal_forward, horizontal_field);
    Some(wrap_degrees(radians_to_degrees(atan2_approx(sine, cosine))))
}

fn atan2_approx(y: f32, x: f32) -> f32 {
    if x == 0.0 && y == 0.0 {
        return 0.0;
    }

    let abs_y = abs_f32(y) + 1.0e-10;
    let (ratio, base) = if x < 0.0 {
        ((x + abs_y) / (abs_y - x), 3.0 * PI / 4.0)
    } else {
        ((x - abs_y) / (x + abs_y), PI / 4.0)
    };
    let angle = base + (0.1963 * ratio * ratio - 0.9817) * ratio;

    if y < 0.0 { -angle } else { angle }
}