//! IMU presentation model.

use embassy_time::{Duration, Instant};

use crate::services::imu;

// Match the 100 Hz fusion publisher instead of imposing a separate 25 Hz UI
// ceiling. The replace-latest input still collapses samples whenever rendering
// is slower than acquisition, so CPU0 always consumes the freshest attitude.
const IMU_UPDATE: Duration = Duration::from_millis(10);

#[derive(Clone, Copy, Debug)]
pub(crate) struct ImuDisplay {
    pub(crate) roll_deg: i32,
    pub(crate) pitch_deg: i32,
    pub(crate) yaw_deg: i32,
    pub(crate) status: imu::Status,
    pub(crate) mag_status: imu::MagStatus,
    pub(crate) mag_field_ut: i32,
    pub(crate) mag_calibration: u8,
}

pub(super) struct Model {
    input: imu::Input,
    display: ImuDisplay,
    last_revision: u32,
    last_update: Instant,
    dirty: bool,
}

impl Model {
    pub(super) fn new(input: imu::Input) -> Self {
        Self {
            input,
            display: ImuDisplay {
                roll_deg: 0,
                pitch_deg: 0,
                yaw_deg: 0,
                status: imu::Status::Starting,
                mag_status: imu::MagStatus::Missing,
                mag_field_ut: 0,
                mag_calibration: 0,
            },
            last_revision: 0,
            last_update: Instant::now(),
            dirty: true,
        }
    }

    pub(super) fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    pub(super) fn update_if_due(&mut self, now: Instant) {
        if now - self.last_update < IMU_UPDATE {
            return;
        }
        self.last_update = now;
        let Some(snapshot) = self.input.take_latest() else {
            return;
        };
        if snapshot.revision == self.last_revision {
            return;
        }
        self.last_revision = snapshot.revision;
        self.display = ImuDisplay {
            roll_deg: round_units(snapshot.orientation.roll_deg),
            pitch_deg: round_units(snapshot.orientation.pitch_deg),
            yaw_deg: round_units(snapshot.orientation.yaw_deg),
            status: snapshot.status,
            mag_status: snapshot.mag_status,
            mag_field_ut: round_units(snapshot.mag_field_ut),
            mag_calibration: snapshot.mag_calibration_percent,
        };
        self.dirty = true;
    }

    pub(super) fn take_display(&mut self) -> Option<ImuDisplay> {
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
