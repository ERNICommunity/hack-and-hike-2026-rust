//! IMU worldview application model.

use embassy_time::{Duration, Instant};

use crate::capabilities::imu;

// Match the 100 Hz fusion publisher instead of imposing a separate 25 Hz UI
// ceiling. The replace-latest capability collapses samples whenever rendering
// is slower than acquisition, so CPU0 always consumes the freshest attitude.
const IMU_UPDATE: Duration = Duration::from_millis(10);

#[derive(Clone, Copy, Debug)]
pub(crate) struct DisplayState {
    /// CPU1 publication revision for diagnostics and stale-sample detection.
    pub(crate) sample_revision: u32,
    /// Euler values are for the numeric header only. World rendering consumes
    /// gravity_screen/north_screen directly and never reconstructs pose from them.
    pub(crate) roll_deg: f32,
    pub(crate) pitch_deg: f32,
    pub(crate) yaw_deg: f32,
    pub(crate) gravity_screen: [f32; 3],
    pub(crate) north_screen: [f32; 3],
    pub(crate) status: imu::Status,
    pub(crate) mag_status: imu::MagStatus,
    pub(crate) mag_field_ut: i32,
    pub(crate) mag_calibration: u8,
}

pub(crate) struct Model {
    imu: imu::Imu,
    display: DisplayState,
    last_revision: u32,
    last_update: Instant,
    logged_calibrated_sample: bool,
    dirty: bool,
}

impl Model {
    pub(crate) fn new(imu: imu::Imu) -> Self {
        Self {
            imu,
            display: DisplayState {
                sample_revision: 0,
                roll_deg: 0.0,
                pitch_deg: 0.0,
                yaw_deg: 0.0,
                gravity_screen: [0.0, 0.0, 1.0],
                north_screen: [1.0, 0.0, 0.0],
                status: imu::Status::Starting,
                mag_status: imu::MagStatus::Missing,
                mag_field_ut: 0,
                mag_calibration: 0,
            },
            last_revision: 0,
            last_update: Instant::now(),
            logged_calibrated_sample: false,
            dirty: true,
        }
    }

    pub(crate) fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    pub(crate) fn update_if_due(&mut self, now: Instant) {
        if now - self.last_update < IMU_UPDATE {
            return;
        }
        self.last_update = now;
        let Some(sample) = self.imu.latest() else {
            return;
        };
        if sample.revision == self.last_revision {
            return;
        }
        self.last_revision = sample.revision;

        if sample.mag_calibration_percent == 100 && !self.logged_calibrated_sample {
            ::log::info!(
                "CPU0 received calibrated IMU sample: revision={} status={:?} mag_status={:?} field={}uT",
                sample.revision,
                sample.status,
                sample.mag_status,
                round_units(sample.mag_field_strength_ut)
            );
            self.logged_calibrated_sample = true;
        }

        self.display = DisplayState {
            sample_revision: sample.revision,
            roll_deg: sample.orientation.roll_deg,
            pitch_deg: sample.orientation.pitch_deg,
            yaw_deg: sample.orientation.yaw_deg,
            gravity_screen: sample.orientation.gravity_screen,
            north_screen: sample.orientation.north_screen,
            status: sample.status,
            mag_status: sample.mag_status,
            mag_field_ut: round_units(sample.mag_field_strength_ut),
            mag_calibration: sample.mag_calibration_percent,
        };
        self.dirty = true;
    }

    pub(crate) fn take_display(&mut self) -> Option<DisplayState> {
        if !self.dirty {
            return None;
        }
        self.dirty = false;
        Some(self.display)
    }
}

fn round_units(value: f32) -> i32 {
    if value >= 0.0 {
        (value + 0.5) as i32
    } else {
        (value - 0.5) as i32
    }
}
