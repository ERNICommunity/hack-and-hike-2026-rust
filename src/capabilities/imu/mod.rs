//! Motion sensing with the IMU (inertial measurement unit): a BMI270
//! accelerometer and gyroscope, with a BMM150 magnetometer connected to it.
//!
//! A CPU1 task reads the accelerometer and the gyroscope 100 times per
//! second. The magnetometer measures 30 times per second. The task learns
//! the offset of the gyroscope while the board lies still. It also learns
//! how the board itself distorts the magnetic field. Then it combines all
//! measurements into an [`Attitude`]: roll, pitch and compass heading. The
//! application gets [`Sample`]s through the [`Imu`] handle.
//!
//! ```ignore
//! if let Some(sample) = imu.latest() {
//!     let tilt = sample.attitude.roll_deg;
//!     if sample.mag_status == MagStatus::Ready {
//!         let heading = sample.attitude.heading_deg;
//!     }
//! }
//! ```
//!
//! # The compass needs calibration
//!
//! After every start, the magnetometer must measure the Earth's field from
//! many directions. Only then is the heading correct. Turn the board slowly
//! in every direction. [`Sample::mag_calibration_percent`] shows the
//! progress, and [`MagStatus::Ready`] means that the calibration is done.
//!
//! When the calibration cannot collect enough directions in about two
//! minutes, it starts again at 0 %. This can happen, for example, when a
//! magnet came close to the board during the calibration. A finished
//! calibration stays until the board restarts.
//!
//! Sensor registers stay private to this module. The math is in
//! `hack_and_hike_core::imu`, where it is tested on the host computer.

mod bmi270;
mod magnetic;
mod runtime;

use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};

use hack_and_hike_core::imu::{Orientation, frames::screen_from_body};

pub(crate) use runtime::spawn;

/// State of the accelerometer and the gyroscope.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// The sensor is being set up. There are no measurements yet.
    Starting,
    /// The sensor works, and the sample has new measurements.
    Running,
    /// The last read failed. The sample repeats the last attitude, and its
    /// measurements are `None`. The next read tries again.
    Degraded,
    /// The setup failed, or ten reads in a row failed. The sensor is set up
    /// again.
    Fault,
}

/// State of the magnetometer. The compass heading depends on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MagStatus {
    /// The magnetometer does not answer, or it sent no new data for one
    /// second. The heading then comes only from the gyroscope, so it slowly
    /// drifts.
    Missing,
    /// The calibration collects measurements. Turn the board in all
    /// directions.
    Learning,
    /// The calibration is done, and the field looks like the Earth's field.
    /// The heading is correct.
    Ready,
    /// For about one second, the field did not look like the Earth's field,
    /// for example because a magnet is close. The magnetometer does not
    /// correct the heading until the field looks normal again.
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
    /// Rotation around the axis through the top edge, in degrees, -180 to
    /// 180. Positive when the right side of the board is lower than the left
    /// side.
    pub roll_deg: f32,
    /// Rotation around the left-right axis, in degrees, -90 to 90. Positive
    /// when the top edge is raised.
    pub pitch_deg: f32,
    /// Compass heading of the top edge, in degrees clockwise from magnetic
    /// north, from 0 up to 360. Only correct when the magnetometer is
    /// [`MagStatus::Ready`].
    pub heading_deg: f32,
    /// Unit vector towards the ground, in the screen frame.
    pub down: [f32; 3],
    /// Unit vector towards magnetic north, in the screen frame, perpendicular
    /// to `down`.
    pub north: [f32; 3],
}

impl Attitude {
    /// Convert the result of the sensor fusion into an attitude in the screen
    /// frame.
    ///
    /// Roll and pitch come from the down vector in the screen frame.
    /// `Orientation::roll_deg` and `Orientation::pitch_deg` are not used,
    /// because they are angles in the sensor's body frame. The heading comes
    /// from `Orientation::yaw_deg`.
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

/// Convert a heading in degrees into the range `0.0..360.0`. For example,
/// -90 becomes 270.
fn heading_0_to_360(degrees: f32) -> f32 {
    let wrapped = degrees % 360.0;
    if wrapped < 0.0 {
        wrapped + 360.0
    } else {
        wrapped
    }
}

/// One IMU sample, as the application gets it.
///
/// Vectors are `[x, y, z]` in the screen frame that [`Attitude`] describes.
///
/// A measurement is `None` when the sensor gave no valid value for this
/// sample. For all three measurements, this means that the sensor is not set
/// up yet or that this read failed. `status` is then not
/// [`Status::Running`]. One failed read does not mean that the sensor is
/// gone. The magnetic field is also `None` while the magnetometer is
/// [`MagStatus::Missing`], or before its first measurement after setup.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    /// Increases by one with every published sample. A gap means that the
    /// application missed samples.
    pub revision: u32,
    /// State of the accelerometer and the gyroscope.
    pub status: Status,
    /// How the board is held: roll, pitch, heading.
    pub attitude: Attitude,
    /// Acceleration including gravity, in m/s². When the board lies flat
    /// with the screen up, this is about `[0, 0, 9.8]`: gravity points into
    /// the screen.
    pub acceleration_m_s2: Option<[f32; 3]>,
    /// Rotation speed around each axis, in degrees per second. This is the
    /// measured value, before the gyroscope offset is removed, so a board
    /// that lies still can show a small value.
    pub angular_velocity_deg_s: Option<[f32; 3]>,
    /// The magnetic field in microtesla (µT), before calibration. The
    /// magnetometer measures only 30 times per second, so several samples
    /// can have the same value.
    pub magnetic_field_ut: Option<[f32; 3]>,
    /// State of the magnetometer. The compass heading depends on it.
    pub mag_status: MagStatus,
    /// Strength of the newest magnetic field, in µT. After calibration, it is
    /// the strength of the calibrated field, which is scaled to about 50 µT.
    /// Before calibration, it is the strength of the raw field. It is 0 until
    /// the first measurement after setup.
    pub mag_field_strength_ut: f32,
    /// Progress of the compass calibration, 0 to 100. 100 means that the
    /// calibration is done. The value goes back to 0 when the calibration
    /// starts again.
    pub mag_calibration_percent: u8,
}

/// The physical measurements of one sample, in the body frame.
///
/// The body frame uses the axes of the BMI270 chip. The sensor fusion does
/// not change these values.
#[derive(Clone, Copy, Debug, Default)]
struct Measurements {
    /// Acceleration including gravity, in m/s². `None` when the read failed
    /// or no read happened.
    acceleration_m_s2: Option<[f32; 3]>,
    /// Rotation speed in degrees per second. `None` when the read failed or
    /// no read happened.
    angular_velocity_deg_s: Option<[f32; 3]>,
    /// Magnetic field before calibration, in microtesla (µT). `None` while
    /// the magnetometer gives no current measurement.
    magnetic_field_ut: Option<[f32; 3]>,
}

impl Measurements {
    /// The same measurements, turned from the sensor's body frame into the
    /// screen frame that [`Attitude`] describes.
    fn in_screen_frame(self) -> Self {
        Self {
            acceleration_m_s2: self.acceleration_m_s2.map(screen_from_body),
            angular_velocity_deg_s: self.angular_velocity_deg_s.map(screen_from_body),
            magnetic_field_ut: self.magnetic_field_ut.map(screen_from_body),
        }
    }
}

/// The state that the CPU1 IMU task and the application share.
struct Service {
    /// The newest sample. A `Signal` holds at most one value. Publishing
    /// replaces a sample that was not read. Taking the sample leaves the
    /// signal empty.
    latest: Signal<CriticalSectionRawMutex, Sample>,
}

/// The only service instance.
///
/// It is a `static`, so both ends can hold a `&'static` reference to it. The
/// runtime needs this reference to use the service from a task on CPU1. The
/// `Signal` uses a critical section, so both CPUs can use it safely.
static SERVICE: Service = Service {
    latest: Signal::new(),
};

/// Application handle for the motion sensors; see the [module docs](self).
///
/// CPU1 publishes about 100 samples per second. Only the newest sample is
/// kept. So an application that reads less often still gets new data, and
/// [`Sample::revision`] shows when it missed samples.
pub struct Imu {
    /// The service that the CPU1 task publishes samples to.
    service: &'static Service,
}

impl Imu {
    /// The newest sample, or `None` when nothing new was published since the
    /// previous call. Never waits.
    pub fn latest(&mut self) -> Option<Sample> {
        self.service.latest.try_take()
    }
}

/// The CPU1 side of the IMU: the IMU task uses it to publish samples.
#[derive(Clone, Copy)]
pub(crate) struct Runtime {
    /// The service that this side publishes samples to.
    service: &'static Service,
}

impl Runtime {
    /// Make `sample` the newest sample. It replaces an older sample that the
    /// application did not read.
    fn publish(self, sample: Sample) {
        self.service.latest.signal(sample);
    }
}

/// The two ends of the sample signal. The board creates them once.
pub(crate) struct Endpoints {
    /// For the application.
    pub(crate) handle: Imu,
    /// For the CPU1 IMU task.
    pub(crate) runtime: Runtime,
}

/// Create both ends of the sample signal.
pub(crate) fn endpoints() -> Endpoints {
    Endpoints {
        handle: Imu { service: &SERVICE },
        runtime: Runtime { service: &SERVICE },
    }
}
