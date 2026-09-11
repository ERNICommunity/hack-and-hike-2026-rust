//! Pure gyro-bias estimation and orientation fusion.
//!
//! This module deliberately has no I2C, Embassy, or sensor-register knowledge.
//!
//! Fusion keeps a complete orientation basis in screen coordinates: the world
//! gravity direction and magnetic-north direction expressed in the rotating
//! device frame. Both vectors are propagated by the gyroscope. Accelerometer
//! correction rotates the complete basis, while magnetometer correction rotates
//! only north around gravity. This avoids the Euler/azimuth singularity that a
//! separate `gravity + scalar yaw` representation has when camera-forward is
//! vertical (display flat on a table).

use super::Orientation;

// Magnetic north is a long-term absolute reference. Keep its per-frame authority
// deliberately small so residual hard/soft-iron error cannot make the compass
// hunt while the gyro already provides smooth short-term motion.
const MAX_MAG_CORRECTION_PER_SAMPLE_DEG: f32 = 0.08;
const MAG_DEADBAND_DEG: f32 = 1.5;
const MAG_DIRECTION_FILTER_ALPHA: f32 = 0.18;
// The 30 Hz BMM150 is delayed relative to the gyro. Only fuse it once hand motion
// is genuinely slow; faster motion is carried by the gyro and corrected later.
const MAG_FUSION_MAX_RATE_DPS: f32 = 20.0;
const MAG_QUIET_SAMPLES_BEFORE_FUSION: u8 = 5;
const MAG_INITIAL_LOCK_SAMPLES: u8 = 5;
const MAX_MAG_INITIAL_ERROR_JITTER_DEG: f32 = 6.0;
const MAG_RECOVERY_MIN_INNOVATION_DEG: f32 = 30.0;
const MAG_RECOVERY_SAMPLES: u8 = 8;
const MAX_MAG_RECOVERY_ERROR_JITTER_DEG: f32 = 6.0;
// Reject only a physically weak horizontal magnetic field. Device attitude is
// intentionally not part of this test: magnetic north remains observable when
// camera-forward is vertical.
const MIN_MAG_HORIZONTAL_FIELD_UT: f32 = 8.0;

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
        let accel_norm_sq = dot3(accel_g, accel_g);
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
    // Inertially fixed world vectors expressed in the rotating screen frame.
    gravity_screen: [f32; 3],
    north_screen: [f32; 3],
    previous_gyro_screen_dps: Option<[f32; 3]>,
    magnetic_locked: bool,
    filtered_magnetic_north: Option<[f32; 3]>,
    quiet_mag_samples: u8,
    recovery_armed: bool,
    pending_mag_error_deg: Option<f32>,
    pending_mag_samples: u8,
    recovery_mag_error_deg: Option<f32>,
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
            gravity_screen: [0.0, 0.0, 1.0],
            north_screen: [1.0, 0.0, 0.0],
            previous_gyro_screen_dps: None,
            magnetic_locked: false,
            filtered_magnetic_north: None,
            quiet_mag_samples: 0,
            recovery_armed: false,
            pending_mag_error_deg: None,
            pending_mag_samples: 0,
            recovery_mag_error_deg: None,
            recovery_mag_samples: 0,
            initialized: false,
        }
    }

    pub(super) fn invalidate_absolute_heading(&mut self) {
        self.magnetic_locked = false;
        self.filtered_magnetic_north = None;
        self.quiet_mag_samples = 0;
        self.recovery_armed = true;
        self.clear_pending_magnetic_candidate();
        self.clear_recovery_candidate();
    }

    pub(super) fn reset_rate_history(&mut self) {
        self.previous_gyro_screen_dps = None;
    }

    fn clear_pending_magnetic_candidate(&mut self) {
        self.pending_mag_error_deg = None;
        self.pending_mag_samples = 0;
    }

    fn clear_recovery_candidate(&mut self) {
        self.recovery_mag_error_deg = None;
        self.recovery_mag_samples = 0;
    }

    fn reset_magnetic_observation_window(&mut self) {
        self.quiet_mag_samples = 0;
        self.filtered_magnetic_north = None;
        self.recovery_armed = true;
        self.clear_pending_magnetic_candidate();
        self.clear_recovery_candidate();
    }

    fn note_motion(&mut self) {
        self.reset_magnetic_observation_window();
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
        let measured_gravity_screen = normalize3(screen_vector_from_body(accel_g));

        if !self.initialized {
            if let Some(gravity) = measured_gravity_screen {
                self.gravity_screen = gravity;
            }
            self.north_screen = initial_horizontal_reference(self.gravity_screen);
            self.initialized = true;
            self.update_euler_output();
            return self.orientation;
        }

        let gyro_screen = screen_vector_from_body(gyro_dps);
        let integration_gyro = self
            .previous_gyro_screen_dps
            .map(|previous| {
                [
                    0.5 * (previous[0] + gyro_screen[0]),
                    0.5 * (previous[1] + gyro_screen[1]),
                    0.5 * (previous[2] + gyro_screen[2]),
                ]
            })
            .unwrap_or(gyro_screen);
        self.previous_gyro_screen_dps = Some(gyro_screen);

        // Propagate the complete orientation basis. Coordinates of an inertially
        // fixed vector in a rotating body obey v_dot = v x omega.
        let predicted_gravity =
            integrate_inertial_vector(self.gravity_screen, integration_gyro, dt_seconds);
        let predicted_north =
            integrate_inertial_vector(self.north_screen, integration_gyro, dt_seconds);

        let accel_norm_sq = dot3(accel_g, accel_g);
        let accel_plausible = (0.75 * 0.75..=1.25 * 1.25).contains(&accel_norm_sq);
        let alpha = clamp_f32(roll_pitch_alpha, 0.0, 1.0);

        if accel_plausible {
            if let Some(measured) = measured_gravity_screen {
                let blended = normalize3([
                    alpha * predicted_gravity[0] + (1.0 - alpha) * measured[0],
                    alpha * predicted_gravity[1] + (1.0 - alpha) * measured[1],
                    alpha * predicted_gravity[2] + (1.0 - alpha) * measured[2],
                ])
                .unwrap_or(predicted_gravity);

                // Apply the same leveling correction to north. Correcting gravity
                // alone and then reconstructing yaw would silently destroy one
                // degree of orientation during arbitrary 3-D movement.
                self.north_screen = rotate_between(predicted_gravity, blended, predicted_north);
                self.gravity_screen = blended;
            } else {
                self.gravity_screen = predicted_gravity;
                self.north_screen = predicted_north;
            }
        } else {
            self.gravity_screen = predicted_gravity;
            self.north_screen = predicted_north;
        }
        self.north_screen = horizontal_unit(self.north_screen, self.gravity_screen)
            .unwrap_or_else(|| initial_horizontal_reference(self.gravity_screen));

        let total_rate_dps = max_abs3(gyro_dps);
        if total_rate_dps > MAG_FUSION_MAX_RATE_DPS {
            self.note_motion();
        } else if let Some(measured_north) = magnetic_field_ut
            .map(screen_vector_from_body)
            .and_then(|field| magnetic_north(field, self.gravity_screen))
        {
            let measured_north = self.filter_magnetic_direction(measured_north);
            self.fuse_magnetic_north(measured_north, yaw_alpha);
        }

        self.update_euler_output();
        self.orientation
    }

    fn update_euler_output(&mut self) {
        let gravity_body = body_vector_from_screen(self.gravity_screen);
        let (roll, pitch) = attitude_from_gravity(gravity_body);
        self.orientation.roll_deg = roll;
        self.orientation.pitch_deg = pitch;

        // Camera-forward azimuth is mathematically undefined only at the exact
        // pole. The full north/gravity basis remains valid there, so retain the
        // last scalar display value for that instant and naturally emerge on the
        // correct branch as soon as forward has a horizontal projection again.
        if let Some(yaw) = heading_from_north(self.north_screen, self.gravity_screen) {
            self.orientation.yaw_deg = yaw;
        }
    }

    fn filter_magnetic_direction(&mut self, measured: [f32; 3]) -> [f32; 3] {
        let filtered = self
            .filtered_magnetic_north
            .and_then(|previous| {
                normalize3([
                    previous[0] + MAG_DIRECTION_FILTER_ALPHA * (measured[0] - previous[0]),
                    previous[1] + MAG_DIRECTION_FILTER_ALPHA * (measured[1] - previous[1]),
                    previous[2] + MAG_DIRECTION_FILTER_ALPHA * (measured[2] - previous[2]),
                ])
            })
            .unwrap_or(measured);
        let filtered = horizontal_unit(filtered, self.gravity_screen).unwrap_or(measured);
        self.filtered_magnetic_north = Some(filtered);
        filtered
    }

    fn fuse_magnetic_north(&mut self, measured_north: [f32; 3], yaw_alpha: f32) {
        self.quiet_mag_samples = self.quiet_mag_samples.saturating_add(1);
        if self.quiet_mag_samples < MAG_QUIET_SAMPLES_BEFORE_FUSION {
            return;
        }

        let error_deg = signed_angle_deg(self.north_screen, measured_north, self.gravity_screen);

        if !self.magnetic_locked {
            let consistent = self
                .pending_mag_error_deg
                .map(|previous| {
                    abs_f32(wrap_degrees(error_deg - previous)) <= MAX_MAG_INITIAL_ERROR_JITTER_DEG
                })
                .unwrap_or(false);

            if consistent {
                self.pending_mag_samples = self.pending_mag_samples.saturating_add(1);
                let previous = self.pending_mag_error_deg.unwrap_or(error_deg);
                self.pending_mag_error_deg = Some(wrap_degrees(
                    previous + 0.25 * wrap_degrees(error_deg - previous),
                ));
            } else {
                self.pending_mag_error_deg = Some(error_deg);
                self.pending_mag_samples = 1;
            }

            if self.pending_mag_samples >= MAG_INITIAL_LOCK_SAMPLES {
                let acquired = self.pending_mag_error_deg.unwrap_or(error_deg);
                self.rotate_north(acquired);
                self.magnetic_locked = true;
                self.recovery_armed = false;
                self.clear_pending_magnetic_candidate();
                self.clear_recovery_candidate();
            }
            return;
        }

        if abs_f32(error_deg) >= MAG_RECOVERY_MIN_INNOVATION_DEG {
            if !self.recovery_armed {
                self.clear_recovery_candidate();
                return;
            }

            let consistent = self
                .recovery_mag_error_deg
                .map(|previous| {
                    abs_f32(wrap_degrees(error_deg - previous))
                        <= MAX_MAG_RECOVERY_ERROR_JITTER_DEG
                })
                .unwrap_or(false);

            if consistent {
                self.recovery_mag_samples = self.recovery_mag_samples.saturating_add(1);
                let previous = self.recovery_mag_error_deg.unwrap_or(error_deg);
                self.recovery_mag_error_deg = Some(wrap_degrees(
                    previous + 0.25 * wrap_degrees(error_deg - previous),
                ));
            } else {
                self.recovery_mag_error_deg = Some(error_deg);
                self.recovery_mag_samples = 1;
            }

            if self.recovery_mag_samples >= MAG_RECOVERY_SAMPLES {
                let recovered = self.recovery_mag_error_deg.unwrap_or(error_deg);
                self.rotate_north(recovered);
                self.recovery_armed = false;
                self.clear_recovery_candidate();
            }
            return;
        }

        self.recovery_armed = false;
        self.clear_recovery_candidate();
        if abs_f32(error_deg) <= MAG_DEADBAND_DEG {
            return;
        }

        let requested = (1.0 - clamp_f32(yaw_alpha, 0.0, 1.0)) * error_deg;
        let applied = clamp_f32(
            requested,
            -MAX_MAG_CORRECTION_PER_SAMPLE_DEG,
            MAX_MAG_CORRECTION_PER_SAMPLE_DEG,
        );
        self.rotate_north(applied);
    }

    fn rotate_north(&mut self, degrees: f32) {
        self.north_screen = rotate_around_axis(
            self.north_screen,
            self.gravity_screen,
            degrees * DEG_TO_RAD,
        );
        self.north_screen = horizontal_unit(self.north_screen, self.gravity_screen)
            .unwrap_or(self.north_screen);
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

fn body_vector_from_screen(value: [f32; 3]) -> [f32; 3] {
    [-value[1], -value[2], value[0]]
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
    if norm_sq < 0.000001 {
        return None;
    }
    let inverse = 1.0 / sqrt_approx(norm_sq);
    Some([value[0] * inverse, value[1] * inverse, value[2] * inverse])
}

fn horizontal_unit(value: [f32; 3], gravity: [f32; 3]) -> Option<[f32; 3]> {
    let along_gravity = dot3(value, gravity);
    normalize3([
        value[0] - gravity[0] * along_gravity,
        value[1] - gravity[1] * along_gravity,
        value[2] - gravity[2] * along_gravity,
    ])
}

fn initial_horizontal_reference(gravity: [f32; 3]) -> [f32; 3] {
    horizontal_unit([1.0, 0.0, 0.0], gravity)
        .or_else(|| horizontal_unit([0.0, 1.0, 0.0], gravity))
        .unwrap_or([0.0, 0.0, 1.0])
}

fn integrate_inertial_vector(
    vector: [f32; 3],
    gyro_dps: [f32; 3],
    dt_seconds: f32,
) -> [f32; 3] {
    let omega = [
        gyro_dps[0] * DEG_TO_RAD,
        gyro_dps[1] * DEG_TO_RAD,
        gyro_dps[2] * DEG_TO_RAD,
    ];
    let derivative = cross3(vector, omega);
    normalize3([
        vector[0] + derivative[0] * dt_seconds,
        vector[1] + derivative[1] * dt_seconds,
        vector[2] + derivative[2] * dt_seconds,
    ])
    .unwrap_or(vector)
}

fn rotate_between(from: [f32; 3], to: [f32; 3], value: [f32; 3]) -> [f32; 3] {
    let axis_raw = cross3(from, to);
    let axis_norm_sq = dot3(axis_raw, axis_raw);
    if axis_norm_sq < 0.000001 {
        return value;
    }
    let axis = normalize3(axis_raw).unwrap_or([0.0, 0.0, 1.0]);
    let sine = sqrt_approx(axis_norm_sq).min(1.0);
    let cosine = clamp_f32(dot3(from, to), -1.0, 1.0);
    rotate_around_axis_sin_cos(value, axis, sine, cosine)
}

fn rotate_around_axis(value: [f32; 3], axis: [f32; 3], angle: f32) -> [f32; 3] {
    rotate_around_axis_sin_cos(value, axis, sin_approx(angle), cos_approx(angle))
}

fn rotate_around_axis_sin_cos(
    value: [f32; 3],
    axis: [f32; 3],
    sine: f32,
    cosine: f32,
) -> [f32; 3] {
    let cross = cross3(axis, value);
    let along = dot3(axis, value) * (1.0 - cosine);
    [
        value[0] * cosine + cross[0] * sine + axis[0] * along,
        value[1] * cosine + cross[1] * sine + axis[1] * along,
        value[2] * cosine + cross[2] * sine + axis[2] * along,
    ]
}

fn attitude_from_gravity(gravity: [f32; 3]) -> (f32, f32) {
    let [gx, gy, gz] = gravity;
    let roll = radians_to_degrees(atan2_approx(gy, gz));
    let pitch = radians_to_degrees(atan2_approx(-gx, sqrt_approx(gy * gy + gz * gz)));
    (roll, pitch)
}

fn magnetic_north(field: [f32; 3], gravity: [f32; 3]) -> Option<[f32; 3]> {
    let field_along_gravity = dot3(field, gravity);
    let horizontal = [
        field[0] - gravity[0] * field_along_gravity,
        field[1] - gravity[1] * field_along_gravity,
        field[2] - gravity[2] * field_along_gravity,
    ];
    let horizontal_sq = dot3(horizontal, horizontal);
    if horizontal_sq < MIN_MAG_HORIZONTAL_FIELD_UT * MIN_MAG_HORIZONTAL_FIELD_UT {
        return None;
    }
    normalize3(horizontal)
}

fn heading_from_north(north: [f32; 3], gravity: [f32; 3]) -> Option<f32> {
    let horizontal_forward = horizontal_unit([1.0, 0.0, 0.0], gravity)?;
    let horizontal_north = horizontal_unit(north, gravity)?;
    let sine = -dot3(gravity, cross3(horizontal_forward, horizontal_north));
    let cosine = dot3(horizontal_forward, horizontal_north);
    Some(wrap_degrees(radians_to_degrees(atan2_approx(sine, cosine))))
}

fn signed_angle_deg(from: [f32; 3], to: [f32; 3], axis: [f32; 3]) -> f32 {
    let sine = dot3(axis, cross3(from, to));
    let cosine = dot3(from, to);
    wrap_degrees(radians_to_degrees(atan2_approx(sine, cosine)))
}

fn sin_approx(value: f32) -> f32 {
    let x = wrap_radians(value);
    let x2 = x * x;
    x * (1.0 - x2 / 6.0 + x2 * x2 / 120.0 - x2 * x2 * x2 / 5040.0)
}

fn cos_approx(value: f32) -> f32 {
    let x = wrap_radians(value);
    let x2 = x * x;
    1.0 - x2 / 2.0 + x2 * x2 / 24.0 - x2 * x2 * x2 / 720.0
}

fn wrap_radians(mut value: f32) -> f32 {
    while value > PI {
        value -= 2.0 * PI;
    }
    while value < -PI {
        value += 2.0 * PI;
    }
    value
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
