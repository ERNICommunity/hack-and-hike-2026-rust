//! Gyro-bias estimation and orientation fusion. Pure math, no hardware.
//!
//! The gyro bias is the rate a gyroscope reports while it does not turn.
//!
//! Fusion keeps two unit vectors in the screen frame: the direction of
//! gravity and the direction of magnetic north. Both are fixed in the world,
//! so they turn in the screen frame when the board turns. Together they
//! describe the complete orientation.
//!
//! - The gyroscope turns both vectors with each sample.
//! - The accelerometer correction turns both vectors together.
//! - The magnetometer correction turns only north, around gravity.
//!
//! Roll, pitch and yaw are computed from the two vectors, only for output.

use core::f32::consts::PI;

use super::{
    Orientation, frames,
    vec3::{self, cross, dot, normalize},
};

/// How strongly each correction changes the vectors that the gyroscope
/// predicted.
#[derive(Clone, Copy)]
pub struct Gains {
    /// Weight of the gyroscope prediction against the accelerometer when
    /// leveling (correcting gravity). 0 uses only the accelerometer; 1 never
    /// levels.
    pub roll_pitch_alpha: f32,
    /// Fraction of the heading error the magnetometer corrects per sample,
    /// before the limits of the magnetic correction apply.
    pub magnetic_gain: f32,
}

impl Gains {
    /// Tuned for the 100 Hz sample rate of the real sensors.
    pub const PRODUCTION: Self = Self {
        roll_pitch_alpha: 0.98,
        magnetic_gain: 0.02,
    };
}

/// Lowest accelerometer magnitude, in g, that counts as "mostly gravity".
/// Only such samples level the orientation.
const ACCEL_PLAUSIBLE_MIN_G: f32 = 0.75;
/// Highest accelerometer magnitude, in g, that counts as "mostly gravity".
const ACCEL_PLAUSIBLE_MAX_G: f32 = 1.25;

// The gyro bias is learned only while the board is clearly still.
/// Lowest accelerometer magnitude, in g, that counts as still.
const STATIONARY_ACCEL_MIN_G: f32 = 0.90;
/// Highest accelerometer magnitude, in g, that counts as still.
const STATIONARY_ACCEL_MAX_G: f32 = 1.10;
/// Every gyroscope axis must read below this, in degrees per second, to
/// count as still. This is the rate before the bias is removed.
const STATIONARY_GYRO_MAX_DPS: f32 = 3.0;
/// Fraction of the way the bias moves towards the measured rate with each
/// still sample, until the estimate is settled.
const BIAS_LEARN_RATE_INITIAL: f32 = 0.02;
/// The same fraction after the estimate is settled. It is ten times smaller,
/// so that noise averages out.
const BIAS_LEARN_RATE_SETTLED: f32 = 0.002;
/// Consecutive still samples after which the bias counts as settled (one
/// second at 100 Hz).
const BIAS_SETTLED_AFTER_SAMPLES: u16 = 100;

// The magnetometer measures only at 30 Hz, and its samples are late and
// noisy. So each magnetic correction is small and limited, and it cannot
// change the short-term orientation much. Fast motion reduces the weight of
// the correction gradually. In the constants below, "rate" is the largest
// absolute gyroscope rate of the three axes, in degrees per second.
/// Largest heading correction per sample, in degrees, in normal operation.
/// The motion weight (see `magnetic_motion_weight`) reduces it further.
const MAX_MAG_CORRECTION_NORMAL_DEG: f32 = 0.12;
/// Largest heading correction per sample, in degrees, while north moves
/// towards a confirmed large error. The motion weight reduces it further.
const MAX_MAG_CORRECTION_RECOVERY_DEG: f32 = 0.75;
/// Factor for `Gains::magnetic_gain` while north moves towards a confirmed
/// large error.
const RECOVERY_GAIN_BOOST: f32 = 4.0;
/// Heading errors up to this many degrees are ignored. This deadband keeps
/// noise from moving north back and forth.
const MAG_DEADBAND_DEG: f32 = 1.5;
/// Weight of the newest measured north direction in the low-pass filter
/// while the board does not turn. A low-pass filter smooths a signal: it
/// keeps slow changes and removes fast noise.
const MAG_DIRECTION_FILTER_ALPHA_SLOW: f32 = 0.18;
/// Weight of the newest measured north direction while the board turns at
/// `MAG_DIRECTION_FILTER_FAST_RATE_DPS` or faster.
const MAG_DIRECTION_FILTER_ALPHA_FAST: f32 = 0.75;
/// Rate, in degrees per second, at which the filter weight reaches
/// `MAG_DIRECTION_FILTER_ALPHA_FAST`. Between 0 and this rate, the weight
/// grows linearly from `MAG_DIRECTION_FILTER_ALPHA_SLOW`.
const MAG_DIRECTION_FILTER_FAST_RATE_DPS: f32 = 240.0;
/// Up to this rate, in degrees per second, the magnetometer corrects
/// heading with full weight.
const MAG_FULL_AUTHORITY_RATE_DPS: f32 = 20.0;
/// At and above this rate, in degrees per second, the weight of the
/// magnetic correction is `MAG_MIN_MOTION_WEIGHT`.
const MAG_LOW_AUTHORITY_RATE_DPS: f32 = 360.0;
/// Smallest weight of the magnetic correction during fast motion.
const MAG_MIN_MOTION_WEIGHT: f32 = 0.08;
/// Consistent heading errors needed before the magnetometer is locked, that
/// is, before it may correct north. This applies at the start and after
/// `Fusion::invalidate_absolute_heading`.
const MAG_INITIAL_LOCK_SAMPLES: u8 = 5;
/// The magnetometer is not locked while the board turns faster than this,
/// in degrees per second.
const MAG_INITIAL_LOCK_MAX_RATE_DPS: f32 = 120.0;
/// A heading error of at least this many degrees is a large innovation
/// (a measurement far from the prediction). It must repeat before it may
/// correct north.
const MAG_LARGE_INNOVATION_DEG: f32 = 30.0;
/// Consistent heading errors in a row that confirm a large innovation.
const MAG_LARGE_CONFIRM_SAMPLES: u8 = 5;
/// A heading error within this many degrees of the smoothed error of the
/// current run counts as consistent.
const MAX_MAG_ERROR_JITTER_DEG: f32 = 8.0;
/// Weight of each new consistent heading error in the smoothed error.
const CANDIDATE_SMOOTHING: f32 = 0.25;
/// Smallest horizontal field strength, in uT, from which north can be
/// measured. A weaker horizontal field gives no north direction.
const MIN_MAG_HORIZONTAL_FIELD_UT: f32 = 8.0;

/// Multiply by this to turn degrees into radians.
const DEG_TO_RAD: f32 = PI / 180.0;

/// Learns the gyroscope's zero-rate offset (the bias) while the board is
/// still.
#[derive(Clone, Copy)]
pub struct GyroBias {
    /// Current bias estimate per body axis, in degrees per second.
    bias_dps: [f32; 3],
    /// Still samples in a row. A sample that is not still sets it to zero.
    stationary_samples: u16,
    /// Set after `BIAS_SETTLED_AFTER_SAMPLES` still samples in a row. From
    /// then on, the slower learning rate is used. It is never cleared.
    settled: bool,
}

impl Default for GyroBias {
    fn default() -> Self {
        Self::new()
    }
}

impl GyroBias {
    /// No bias learned yet: rates pass through unchanged at first.
    pub const fn new() -> Self {
        Self {
            bias_dps: [0.0; 3],
            stationary_samples: 0,
            settled: false,
        }
    }

    /// Update the bias estimate and return the bias-corrected rate.
    ///
    /// `accel_g` is in g and `gyro_dps` in degrees per second, both in the
    /// body frame. The result is in degrees per second, in the body frame.
    pub fn correct(&mut self, accel_g: [f32; 3], gyro_dps: [f32; 3]) -> [f32; 3] {
        let accel_norm = vec3::norm(accel_g);
        let stationary = (STATIONARY_ACCEL_MIN_G..=STATIONARY_ACCEL_MAX_G).contains(&accel_norm)
            && vec3::max_abs(gyro_dps) < STATIONARY_GYRO_MAX_DPS;

        if stationary {
            self.stationary_samples = self.stationary_samples.saturating_add(1);
            let rate = if self.settled {
                BIAS_LEARN_RATE_SETTLED
            } else {
                BIAS_LEARN_RATE_INITIAL
            };
            self.bias_dps = vec3::lerp(self.bias_dps, gyro_dps, rate);
            if self.stationary_samples >= BIAS_SETTLED_AFTER_SAMPLES {
                self.settled = true;
            }
        } else {
            self.stationary_samples = 0;
        }

        vec3::sub(gyro_dps, self.bias_dps)
    }
}

/// A run of consistent heading errors. A heading error must repeat before
/// fusion trusts it, so one bad magnetometer sample cannot move north.
#[derive(Clone, Copy, Default)]
struct SmoothedCandidate {
    /// Smoothed error of the current run, in degrees. `None` when there is
    /// no run.
    error_deg: Option<f32>,
    /// Consistent heading errors in the current run.
    samples: u8,
}

impl SmoothedCandidate {
    /// Add a new heading error to the run. Returns how many consistent
    /// errors in a row the run now has.
    ///
    /// An error that is not consistent with the smoothed error (see
    /// `MAX_MAG_ERROR_JITTER_DEG`) starts a new run with this error.
    fn observe(&mut self, error_deg: f32) -> u8 {
        match self.error_deg {
            Some(previous)
                if wrap_degrees(error_deg - previous).abs() <= MAX_MAG_ERROR_JITTER_DEG =>
            {
                self.samples = self.samples.saturating_add(1);
                self.error_deg = Some(wrap_degrees(
                    previous + CANDIDATE_SMOOTHING * wrap_degrees(error_deg - previous),
                ));
            }
            _ => {
                self.error_deg = Some(error_deg);
                self.samples = 1;
            }
        }
        self.samples
    }

    /// Forget the current run.
    fn clear(&mut self) {
        *self = Self::default();
    }
}

/// The orientation filter: feed it one sample at a time with
/// [`Fusion::update`].
///
/// The gyroscope predicts how the board turned since the last sample. The
/// accelerometer moves the prediction towards gravity (it levels it). A
/// calibrated magnetometer moves it towards magnetic north.
#[derive(Clone, Copy)]
pub struct Fusion {
    /// Correction strengths, fixed at construction.
    gains: Gains,
    // Directions that are fixed in the world, given in the screen frame,
    // which turns with the board.
    /// Unit vector of gravity (down).
    gravity_screen: [f32; 3],
    /// Unit vector of magnetic north, always perpendicular to gravity. It
    /// starts as some horizontal direction. Once the magnetometer is locked,
    /// it turns towards the measured north.
    north_screen: [f32; 3],
    /// Last defined heading, in degrees. It is kept while screen x (the
    /// camera direction) points straight up or down, where the heading is
    /// not defined.
    yaw_deg: f32,
    /// Gyroscope rate of the previous sample in the screen frame, in degrees
    /// per second, for trapezoidal integration (see `update`). `None` at the
    /// start and after `reset_rate_history`.
    previous_gyro_screen_dps: Option<[f32; 3]>,
    /// Whether the magnetometer is locked, that is, whether it may correct
    /// north. Cleared by `invalidate_absolute_heading`.
    magnetic_locked: bool,
    /// Output of the low-pass filter on the measured north direction. `None`
    /// before the first measurement and after `invalidate_absolute_heading`.
    filtered_magnetic_north: Option<[f32; 3]>,
    /// Collects consistent heading errors until the magnetometer is locked.
    initial_lock: SmoothedCandidate,
    /// Collects consistent large heading errors until they may correct
    /// north.
    large_innovation: SmoothedCandidate,
    /// Set by the first `update`. That update only takes gravity from the
    /// accelerometer.
    initialized: bool,
}

impl Default for Fusion {
    fn default() -> Self {
        Self::new()
    }
}

impl Fusion {
    /// Fusion with the production gains.
    pub const fn new() -> Self {
        Self::with_gains(Gains::PRODUCTION)
    }

    /// Fusion with custom gains, for tests and tuning.
    pub const fn with_gains(gains: Gains) -> Self {
        Self {
            gains,
            gravity_screen: [0.0, 0.0, 1.0],
            north_screen: [1.0, 0.0, 0.0],
            yaw_deg: 0.0,
            previous_gyro_screen_dps: None,
            magnetic_locked: false,
            filtered_magnetic_north: None,
            initial_lock: SmoothedCandidate {
                error_deg: None,
                samples: 0,
            },
            large_innovation: SmoothedCandidate {
                error_deg: None,
                samples: 0,
            },
            initialized: false,
        }
    }

    /// Forget the magnetic lock. Call it after an event that made the
    /// gyroscope integration unreliable. The magnetometer must then lock
    /// again before it corrects north. The current north is kept until then.
    pub fn invalidate_absolute_heading(&mut self) {
        self.magnetic_locked = false;
        self.filtered_magnetic_north = None;
        self.initial_lock.clear();
        self.large_innovation.clear();
    }

    /// Forget the previous gyroscope rate. Call it after samples were lost,
    /// so the next integration step does not average the rates before and
    /// after the gap.
    pub fn reset_rate_history(&mut self) {
        self.previous_gyro_screen_dps = None;
    }

    /// Advance by one sample and return the new orientation.
    ///
    /// The first call only takes gravity from the accelerometer. The vectors
    /// are in the body frame:
    ///
    /// - `accel_g`: acceleration in g.
    /// - `gyro_dps`: bias-corrected rotation rate in degrees per second.
    /// - `dt_seconds`: time since the previous sample.
    /// - `magnetic_field_ut`: a calibrated field in uT that can be trusted,
    ///   or `None` when there is none.
    pub fn update(
        &mut self,
        accel_g: [f32; 3],
        gyro_dps: [f32; 3],
        dt_seconds: f32,
        magnetic_field_ut: Option<[f32; 3]>,
    ) -> Orientation {
        let measured_gravity = normalize(frames::screen_from_body(accel_g));

        if !self.initialized {
            if let Some(gravity) = measured_gravity {
                self.gravity_screen = gravity;
            }
            self.north_screen = initial_horizontal_reference(self.gravity_screen);
            self.initialized = true;
            return self.orientation();
        }

        // Trapezoidal integration: use the average of the previous and the
        // current rate for this step.
        let gyro_screen = frames::screen_from_body(gyro_dps);
        let integration_gyro = self
            .previous_gyro_screen_dps
            .map_or(gyro_screen, |previous| {
                vec3::lerp(previous, gyro_screen, 0.5)
            });
        self.previous_gyro_screen_dps = Some(gyro_screen);

        // A vector that is fixed in the world changes in the turning screen
        // frame as `v_dot = v x omega`. `v_dot` is its rate of change,
        // `omega` the rotation rate and `x` the cross product.
        let predicted_gravity =
            integrate_inertial_vector(self.gravity_screen, integration_gyro, dt_seconds);
        let predicted_north =
            integrate_inertial_vector(self.north_screen, integration_gyro, dt_seconds);

        let accel_norm = vec3::norm(accel_g);
        let accel_plausible = (ACCEL_PLAUSIBLE_MIN_G..=ACCEL_PLAUSIBLE_MAX_G).contains(&accel_norm);
        match measured_gravity {
            Some(measured) if accel_plausible => {
                let blended = normalize(vec3::lerp(
                    measured,
                    predicted_gravity,
                    self.gains.roll_pitch_alpha,
                ))
                .unwrap_or(predicted_gravity);
                // Turn north by the same leveling correction. If only
                // gravity turned, the heading would change without notice
                // during 3-D motion.
                self.north_screen = rotate_between(predicted_gravity, blended, predicted_north);
                self.gravity_screen = blended;
            }
            _ => {
                self.gravity_screen = predicted_gravity;
                self.north_screen = predicted_north;
            }
        }
        self.north_screen = horizontal_unit(self.north_screen, self.gravity_screen)
            .unwrap_or_else(|| initial_horizontal_reference(self.gravity_screen));

        let rate_dps = vec3::max_abs(gyro_dps);
        if let Some(measured_north) = magnetic_field_ut
            .map(frames::screen_from_body)
            .and_then(|field| magnetic_north(field, self.gravity_screen))
        {
            let measured_north = self.filter_magnetic_direction(measured_north, rate_dps);
            self.fuse_magnetic_north(measured_north, rate_dps);
        }

        self.orientation()
    }

    /// The current basis as an [`Orientation`]. Also stores the heading in
    /// `yaw_deg` when it is defined.
    fn orientation(&mut self) -> Orientation {
        let (roll_deg, pitch_deg) =
            roll_pitch_from_gravity(frames::body_from_screen(self.gravity_screen));
        if let Some(yaw) = heading_from_north(self.north_screen, self.gravity_screen) {
            self.yaw_deg = yaw;
        }
        Orientation {
            roll_deg,
            pitch_deg,
            yaw_deg: self.yaw_deg,
            gravity_screen: self.gravity_screen,
            north_screen: self.north_screen,
        }
    }

    /// Smooth the measured north direction with a low-pass filter and return
    /// the result, made horizontal. At a high rotation rate the newest sample
    /// gets more weight, so that the filter adds little delay.
    fn filter_magnetic_direction(&mut self, measured: [f32; 3], rate_dps: f32) -> [f32; 3] {
        let rate_ratio = (rate_dps / MAG_DIRECTION_FILTER_FAST_RATE_DPS).clamp(0.0, 1.0);
        let alpha = MAG_DIRECTION_FILTER_ALPHA_SLOW
            + (MAG_DIRECTION_FILTER_ALPHA_FAST - MAG_DIRECTION_FILTER_ALPHA_SLOW) * rate_ratio;
        let filtered = self
            .filtered_magnetic_north
            .and_then(|previous| normalize(vec3::lerp(previous, measured, alpha)))
            .unwrap_or(measured);
        let filtered = horizontal_unit(filtered, self.gravity_screen).unwrap_or(measured);
        self.filtered_magnetic_north = Some(filtered);
        filtered
    }

    /// Turn north a small step towards a measured north direction.
    ///
    /// There is no step before the magnetometer is locked, while a large
    /// error is not yet confirmed, or when the error is inside the deadband.
    /// The rotation rate limits the size of the step.
    fn fuse_magnetic_north(&mut self, measured_north: [f32; 3], rate_dps: f32) {
        let error_deg = signed_angle_deg(self.north_screen, measured_north, self.gravity_screen);

        // Do not lock while the board turns fast: then the 30 Hz magnetometer
        // samples are too late. After the lock, fast motion alone makes the
        // magnetic correction smaller, but never zero.
        if !self.magnetic_locked {
            if rate_dps > MAG_INITIAL_LOCK_MAX_RATE_DPS {
                self.initial_lock.clear();
                return;
            }
            if self.initial_lock.observe(error_deg) < MAG_INITIAL_LOCK_SAMPLES {
                return;
            }
            self.magnetic_locked = true;
            self.initial_lock.clear();
        }

        // A large innovation must repeat before it may correct north. This
        // ignores short wrong directions during a fast hand movement. After
        // the confirmation, north turns in small steps, not in one jump.
        let large_error = error_deg.abs() >= MAG_LARGE_INNOVATION_DEG;
        if large_error {
            if self.large_innovation.observe(error_deg) < MAG_LARGE_CONFIRM_SAMPLES {
                return;
            }
        } else {
            self.large_innovation.clear();
        }

        if error_deg.abs() <= MAG_DEADBAND_DEG {
            return;
        }

        let motion_weight = magnetic_motion_weight(rate_dps);
        let (gain, max_step) = if large_error {
            (
                self.gains.magnetic_gain * RECOVERY_GAIN_BOOST,
                MAX_MAG_CORRECTION_RECOVERY_DEG,
            )
        } else {
            (self.gains.magnetic_gain, MAX_MAG_CORRECTION_NORMAL_DEG)
        };
        let max_step = max_step * motion_weight;
        let step_deg = (gain * error_deg * motion_weight).clamp(-max_step, max_step);
        self.rotate_north(step_deg);
    }

    /// Rotate north around gravity by `degrees`, keeping it horizontal.
    fn rotate_north(&mut self, degrees: f32) {
        let rotated =
            rotate_around_axis(self.north_screen, self.gravity_screen, degrees * DEG_TO_RAD);
        self.north_screen = horizontal_unit(rotated, self.gravity_screen).unwrap_or(rotated);
    }
}

/// Weight of the magnetic heading correction at a rotation rate in degrees
/// per second. It is 1 up to `MAG_FULL_AUTHORITY_RATE_DPS`. Then it falls
/// linearly to `MAG_MIN_MOTION_WEIGHT` at `MAG_LOW_AUTHORITY_RATE_DPS` and
/// stays there.
fn magnetic_motion_weight(rate_dps: f32) -> f32 {
    let t = ((rate_dps - MAG_FULL_AUTHORITY_RATE_DPS)
        / (MAG_LOW_AUTHORITY_RATE_DPS - MAG_FULL_AUTHORITY_RATE_DPS))
        .clamp(0.0, 1.0);
    1.0 - t * (1.0 - MAG_MIN_MOTION_WEIGHT)
}

/// The same angle in degrees, moved into the range -180 to 180.
fn wrap_degrees(value: f32) -> f32 {
    let wrapped = value % 360.0;
    if wrapped > 180.0 {
        wrapped - 360.0
    } else if wrapped < -180.0 {
        wrapped + 360.0
    } else {
        wrapped
    }
}

/// The part of `value` perpendicular to the unit vector `gravity`, as a unit
/// vector. `None` when that part is almost zero, that is, when `value`
/// points along gravity.
fn horizontal_unit(value: [f32; 3], gravity: [f32; 3]) -> Option<[f32; 3]> {
    normalize(vec3::sub(value, vec3::scale(gravity, dot(value, gravity))))
}

/// A horizontal unit vector to start north with, before the magnetometer is
/// locked. It is screen x made horizontal, or screen y when screen x points
/// along gravity.
fn initial_horizontal_reference(gravity: [f32; 3]) -> [f32; 3] {
    horizontal_unit([1.0, 0.0, 0.0], gravity)
        .or_else(|| horizontal_unit([0.0, 1.0, 0.0], gravity))
        .unwrap_or([0.0, 0.0, 1.0])
}

/// Advance a unit vector that is fixed in the world by one gyroscope step of
/// `dt_seconds`, with `v_dot = v x omega`. The result is scaled back to
/// length 1. `gyro_dps` is in degrees per second.
fn integrate_inertial_vector(vector: [f32; 3], gyro_dps: [f32; 3], dt_seconds: f32) -> [f32; 3] {
    let omega = vec3::scale(gyro_dps, DEG_TO_RAD);
    normalize(vec3::add(
        vector,
        vec3::scale(cross(vector, omega), dt_seconds),
    ))
    .unwrap_or(vector)
}

/// Apply to `value` the rotation that turns the unit vector `from` onto the
/// unit vector `to`. `value` is returned unchanged when `from` and `to` are
/// (almost) parallel or opposite, because then the rotation axis is not
/// defined.
fn rotate_between(from: [f32; 3], to: [f32; 3], value: [f32; 3]) -> [f32; 3] {
    let axis_raw = cross(from, to);
    let Some(axis) = normalize(axis_raw) else {
        return value;
    };
    let sine = vec3::norm(axis_raw).min(1.0);
    let cosine = dot(from, to).clamp(-1.0, 1.0);
    rotate_around_axis_sin_cos(value, axis, sine, cosine)
}

/// Rotate `value` by `angle` radians around the unit vector `axis`.
fn rotate_around_axis(value: [f32; 3], axis: [f32; 3], angle: f32) -> [f32; 3] {
    rotate_around_axis_sin_cos(value, axis, libm::sinf(angle), libm::cosf(angle))
}

/// Rotate `value` around the unit vector `axis` by the angle with the given
/// sine and cosine. This is Rodrigues' rotation formula.
fn rotate_around_axis_sin_cos(value: [f32; 3], axis: [f32; 3], sine: f32, cosine: f32) -> [f32; 3] {
    let along = vec3::scale(axis, dot(axis, value) * (1.0 - cosine));
    vec3::add(
        vec3::add(
            vec3::scale(value, cosine),
            vec3::scale(cross(axis, value), sine),
        ),
        along,
    )
}

/// Roll and pitch in degrees from the gravity direction in the body frame.
fn roll_pitch_from_gravity(gravity: [f32; 3]) -> (f32, f32) {
    let [gx, gy, gz] = gravity;
    let roll = libm::atan2f(gy, gz).to_degrees();
    let pitch = libm::atan2f(-gx, libm::sqrtf(gy * gy + gz * gz)).to_degrees();
    (roll, pitch)
}

/// Horizontal unit vector towards magnetic north, from a magnetic field in
/// uT. `None` when the horizontal part of the field is weaker than
/// `MIN_MAG_HORIZONTAL_FIELD_UT`.
fn magnetic_north(field: [f32; 3], gravity: [f32; 3]) -> Option<[f32; 3]> {
    let horizontal = vec3::sub(field, vec3::scale(gravity, dot(field, gravity)));
    (vec3::norm(horizontal) >= MIN_MAG_HORIZONTAL_FIELD_UT).then(|| normalize(horizontal))?
}

/// Heading in degrees, -180 to 180: the angle from screen x (the camera
/// direction) to north, both made horizontal. `None` when screen x points
/// straight up or down.
fn heading_from_north(north: [f32; 3], gravity: [f32; 3]) -> Option<f32> {
    let forward = horizontal_unit([1.0, 0.0, 0.0], gravity)?;
    let north = horizontal_unit(north, gravity)?;
    let sine = -dot(gravity, cross(forward, north));
    let cosine = dot(forward, north);
    Some(wrap_degrees(libm::atan2f(sine, cosine).to_degrees()))
}

/// Angle in degrees, -180 to 180, from `from` to `to`, measured around
/// `axis` with the right-hand rule.
fn signed_angle_deg(from: [f32; 3], to: [f32; 3], axis: [f32; 3]) -> f32 {
    let sine = dot(axis, cross(from, to));
    let cosine = dot(from, to);
    wrap_degrees(libm::atan2f(sine, cosine).to_degrees())
}
