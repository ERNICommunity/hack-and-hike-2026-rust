//! Motion sensing: BMI270 accelerometer/gyroscope plus BMM150 magnetometer.
//!
//! CPU1 reads the sensors, learns the gyroscope bias and the magnetometer's
//! enclosure distortion, and fuses everything into an orientation. CPU0
//! receives [`Sample`]s through the [`Imu`] handle. Sensor registers stay
//! private to this module; the math lives in `hack_and_hike_core::imu`, where
//! it is tested on the host.

mod bmi270;
mod magnetic;
mod runtime;

use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};

use hack_and_hike_core::imu::{Orientation, frames::screen_from_body};

pub(crate) use runtime::spawn;

/// Health of the accelerometer/gyroscope acquisition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// The sensor is being initialized; no measurement yet.
    Starting,
    /// Samples are flowing.
    Running,
    /// A read failed; the last orientation is being repeated while retrying.
    Degraded,
    /// Repeated failures; the sensor is being re-initialized.
    Fault,
}

/// Health of the magnetometer, which the heading depends on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MagStatus {
    /// No magnetometer is answering; heading drifts with the gyroscope.
    Missing,
    /// Calibration is collecting samples; move the board in all directions.
    Learning,
    /// Calibrated and delivering plausible fields.
    Ready,
    /// The field does not look like the Earth's; heading is not corrected.
    Disturbed,
}

/// How the board is held, in the screen frame.
///
/// The screen frame has `x` pointing out of the top edge of the board (the
/// camera direction), `y` pointing right across the screen and `z` pointing
/// into the screen. A board lying flat on a table, screen up, has roll and
/// pitch 0.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Attitude {
    /// Rotation about the top-edge axis: positive when the right side of the
    /// board is lower than the left.
    pub roll_deg: f32,
    /// Rotation about the left-right axis: positive when the top edge is
    /// raised.
    pub pitch_deg: f32,
    /// Compass heading of the top edge, 0 to 360 degrees clockwise from
    /// magnetic north. Meaningless until [`MagStatus::Ready`].
    pub heading_deg: f32,
    /// Unit vector towards the ground, in the screen frame.
    pub down: [f32; 3],
    /// Unit vector towards magnetic north, in the screen frame, perpendicular
    /// to `down`.
    pub north: [f32; 3],
}

impl Attitude {
    /// Roll, pitch and heading are derived here, once, from the fused basis;
    /// the Euler angles inside `Orientation` follow the sensor's body frame.
    fn from_orientation(orientation: &Orientation) -> Self {
        let [down_x, down_y, down_z] = orientation.gravity_screen;
        Self {
            roll_deg: libm::atan2f(down_y, down_z).to_degrees(),
            pitch_deg: libm::atan2f(-down_x, libm::hypotf(down_y, down_z)).to_degrees(),
            heading_deg: heading_0_to_360(orientation.yaw_deg),
            down: orientation.gravity_screen,
            north: orientation.north_screen,
        }
    }
}

impl Default for Attitude {
    fn default() -> Self {
        Self::from_orientation(&Orientation::default())
    }
}

/// Wrap a heading in degrees into `0.0..360.0`.
fn heading_0_to_360(degrees: f32) -> f32 {
    let wrapped = degrees % 360.0;
    if wrapped < 0.0 {
        wrapped + 360.0
    } else {
        wrapped
    }
}

/// One IMU sample as published to the application.
///
/// Acceleration is in m/s², angular velocity in degrees/s, the magnetic field
/// in microtesla, all in the screen frame described on [`Attitude`]. A `None`
/// measurement means that sensor delivered nothing valid in the current
/// session.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    /// Increments with every published sample; gaps mean the application
    /// skipped samples.
    pub revision: u32,
    pub status: Status,
    pub attitude: Attitude,
    pub acceleration_m_s2: Option<[f32; 3]>,
    pub angular_velocity_deg_s: Option<[f32; 3]>,
    pub magnetic_field_ut: Option<[f32; 3]>,
    pub mag_status: MagStatus,
    /// Magnitude of the (calibrated) magnetic field.
    pub mag_field_strength_ut: f32,
    /// Magnetometer calibration progress, 100 once a model is in use.
    pub mag_calibration_percent: u8,
}

/// Physical measurements of one sample before fusion, in the body frame.
#[derive(Clone, Copy, Debug, Default)]
struct Measurements {
    acceleration_m_s2: Option<[f32; 3]>,
    angular_velocity_deg_s: Option<[f32; 3]>,
    magnetic_field_ut: Option<[f32; 3]>,
}

impl Measurements {
    fn in_screen_frame(self) -> Self {
        Self {
            acceleration_m_s2: self.acceleration_m_s2.map(screen_from_body),
            angular_velocity_deg_s: self.angular_velocity_deg_s.map(screen_from_body),
            magnetic_field_ut: self.magnetic_field_ut.map(screen_from_body),
        }
    }
}

struct Service {
    latest: Signal<CriticalSectionRawMutex, Sample>,
}

static SERVICE: Service = Service {
    latest: Signal::new(),
};

/// Application handle for the motion sensors.
///
/// CPU1 publishes about 100 samples per second. Only the newest one is kept,
/// so an application that polls slower than that always sees fresh data and
/// can detect skipped samples through [`Sample::revision`].
pub struct Imu {
    service: &'static Service,
}

impl Imu {
    /// The newest sample, or `None` when nothing new was published since the
    /// previous call.
    pub fn latest(&mut self) -> Option<Sample> {
        self.service.latest.try_take()
    }
}

/// CPU1 side of the signal.
#[derive(Clone, Copy)]
pub(crate) struct Runtime {
    service: &'static Service,
}

impl Runtime {
    fn publish(self, sample: Sample) {
        self.service.latest.signal(sample);
    }
}

pub(crate) struct Endpoints {
    pub(crate) handle: Imu,
    pub(crate) runtime: Runtime,
}

pub(crate) fn endpoints() -> Endpoints {
    Endpoints {
        handle: Imu { service: &SERVICE },
        runtime: Runtime { service: &SERVICE },
    }
}
