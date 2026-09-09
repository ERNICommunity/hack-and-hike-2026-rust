//! BMM150 magnetometer data decoding, factory compensation, and runtime calibration.
//!
//! Hardware transport lives in `imu.rs` because the BMM150 is reached through
//! the BMI270 auxiliary I²C interface. This module is deliberately pure: it
//! converts the eight auxiliary data bytes plus Bosch factory trim into µT and
//! maintains a small allocation-free hard/soft-iron calibration estimate.
//!
//! The compensation equations are derived from Bosch Sensortec's BSD-3-Clause
//! BMM150 SensorAPI v2.0.0.

const OVERFLOW_XY: i16 = -4096;
const OVERFLOW_Z: i16 = -16384;

/// Plausibility window used while learning calibration. The CoreS3 family can
/// have a large stable hard-iron offset from nearby hardware (speaker, chassis,
/// etc.), so calibration must accept fields much larger than Earth's field and
/// remove the offset before judging magnetic health.
const LEARNING_FIELD_MIN_UT: f32 = 5.0;
const LEARNING_FIELD_MAX_UT: f32 = 2000.0;
/// Normal field-strength window after calibration. Samples outside this range
/// are treated as magnetically disturbed and are not used for yaw correction.
pub const GOOD_FIELD_MIN_UT: f32 = 15.0;
pub const GOOD_FIELD_MAX_UT: f32 = 100.0;
const CALIBRATION_TARGET_SPAN_UT: f32 = 35.0;
const CALIBRATION_MIN_SAMPLES: u16 = 120;

#[derive(Clone, Copy, Debug)]
pub struct Trim {
    dig_x1: i8,
    dig_y1: i8,
    dig_x2: i8,
    dig_y2: i8,
    dig_z1: u16,
    dig_z2: i16,
    dig_z3: i16,
    dig_z4: i16,
    dig_xy1: u8,
    dig_xy2: i8,
    dig_xyz1: u16,
}

impl Trim {
    /// Construct factory trim from the three register blocks Bosch documents:
    /// 0x5D..0x5E, 0x62..0x65, and 0x68..0x71.
    pub fn from_registers(x1_y1: [u8; 2], z4_x2_y2: [u8; 4], z2_to_xy1: [u8; 10]) -> Self {
        Self {
            dig_x1: x1_y1[0] as i8,
            dig_y1: x1_y1[1] as i8,
            dig_x2: z4_x2_y2[2] as i8,
            dig_y2: z4_x2_y2[3] as i8,
            dig_z1: u16::from_le_bytes([z2_to_xy1[2], z2_to_xy1[3]]),
            dig_z2: i16::from_le_bytes([z2_to_xy1[0], z2_to_xy1[1]]),
            dig_z3: i16::from_le_bytes([z2_to_xy1[6], z2_to_xy1[7]]),
            dig_z4: i16::from_le_bytes([z4_x2_y2[0], z4_x2_y2[1]]),
            dig_xy1: z2_to_xy1[9],
            dig_xy2: z2_to_xy1[8] as i8,
            dig_xyz1: u16::from_le_bytes([z2_to_xy1[4], z2_to_xy1[5] & 0x7f]),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Sample {
    pub field_ut: [f32; 3],
    pub field_strength_ut: f32,
    pub data_ready: bool,
}

/// Decode and apply Bosch factory compensation to the BMM150's 8-byte data
/// frame (X, Y, Z and RHALL). Returns `None` for overflow/invalid trim data.
pub fn compensate(data: [u8; 8], trim: Trim) -> Option<Sample> {
    let raw_x = i16::from_le_bytes([data[0], data[1]]) >> 3;
    let raw_y = i16::from_le_bytes([data[2], data[3]]) >> 3;
    let raw_z = i16::from_le_bytes([data[4], data[5]]) >> 1;
    let rhall = u16::from_le_bytes([data[6], data[7]]) >> 2;
    let data_ready = data[6] & 0x01 != 0;

    if raw_x == OVERFLOW_XY
        || raw_y == OVERFLOW_XY
        || raw_z == OVERFLOW_Z
        || rhall == 0
        || trim.dig_xyz1 == 0
        || trim.dig_z1 == 0
        || trim.dig_z2 == 0
    {
        return None;
    }

    let x = compensate_xy(raw_x, rhall, trim.dig_x1, trim.dig_x2, trim)?;
    let y = compensate_xy(raw_y, rhall, trim.dig_y1, trim.dig_y2, trim)?;
    let z = compensate_z(raw_z, rhall, trim)?;
    let field_strength_ut = sqrt_approx(x * x + y * y + z * z);

    Some(Sample {
        field_ut: [x, y, z],
        field_strength_ut,
        data_ready,
    })
}

fn compensate_xy(raw: i16, rhall: u16, dig_1: i8, dig_2: i8, trim: Trim) -> Option<f32> {
    if raw == OVERFLOW_XY || rhall == 0 || trim.dig_xyz1 == 0 {
        return None;
    }

    let x0 = f32::from(trim.dig_xyz1) * 16384.0 / f32::from(rhall);
    let ratio = x0 - 16384.0;
    let x1 = f32::from(trim.dig_xy2) * (ratio * ratio / 268_435_456.0);
    let x2 = x1 + ratio * f32::from(trim.dig_xy1) / 16384.0;
    let x3 = f32::from(dig_2) + 160.0;
    let x4 = f32::from(raw) * ((x2 + 256.0) * x3);
    Some(((x4 / 8192.0) + f32::from(dig_1) * 8.0) / 16.0)
}

fn compensate_z(raw: i16, rhall: u16, trim: Trim) -> Option<f32> {
    if raw == OVERFLOW_Z
        || trim.dig_z2 == 0
        || trim.dig_z1 == 0
        || trim.dig_xyz1 == 0
        || rhall == 0
    {
        return None;
    }

    let z0 = f32::from(raw) - f32::from(trim.dig_z4);
    let z1 = f32::from(rhall) - f32::from(trim.dig_xyz1);
    let z2 = f32::from(trim.dig_z3) * z1;
    let z3 = f32::from(trim.dig_z1) * f32::from(rhall) / 32768.0;
    let z4 = f32::from(trim.dig_z2) + z3;
    if z4 > -0.0001 && z4 < 0.0001 {
        return None;
    }
    let z5 = z0 * 131072.0 - z2;
    Some((z5 / (z4 * 4.0)) / 16.0)
}

/// Online min/max calibration. It learns hard-iron center and a simple diagonal
/// soft-iron scale from normal device motion without allocating or retaining a
/// sample history. Calibration resets at boot; persistence can be added later
/// once the product has a settings/NVS owner.
#[derive(Clone, Copy)]
pub struct Calibration {
    min: [f32; 3],
    max: [f32; 3],
    samples: u16,
}

impl Calibration {
    pub const fn new() -> Self {
        Self {
            min: [f32::MAX; 3],
            max: [f32::MIN; 3],
            samples: 0,
        }
    }

    pub fn observe(&mut self, field_ut: [f32; 3]) {
        let strength = vector_length(field_ut);
        if !(LEARNING_FIELD_MIN_UT..=LEARNING_FIELD_MAX_UT).contains(&strength) {
            return;
        }

        for axis in 0..3 {
            self.min[axis] = min_f32(self.min[axis], field_ut[axis]);
            self.max[axis] = max_f32(self.max[axis], field_ut[axis]);
        }
        self.samples = self.samples.saturating_add(1);
    }

    pub fn progress_percent(&self) -> u8 {
        if self.samples == 0 {
            return 0;
        }
        let span = self.minimum_span();
        let span_progress = clamp_f32(span / CALIBRATION_TARGET_SPAN_UT, 0.0, 1.0);
        let sample_progress = clamp_f32(
            f32::from(self.samples) / f32::from(CALIBRATION_MIN_SAMPLES),
            0.0,
            1.0,
        );
        (100.0 * min_f32(span_progress, sample_progress)) as u8
    }

    pub fn is_ready(&self) -> bool {
        self.samples >= CALIBRATION_MIN_SAMPLES && self.minimum_span() >= CALIBRATION_TARGET_SPAN_UT
    }

    pub fn apply(&self, field_ut: [f32; 3]) -> [f32; 3] {
        if !self.is_ready() {
            return field_ut;
        }

        let radii = [
            (self.max[0] - self.min[0]) * 0.5,
            (self.max[1] - self.min[1]) * 0.5,
            (self.max[2] - self.min[2]) * 0.5,
        ];
        if radii[0] < 0.001 || radii[1] < 0.001 || radii[2] < 0.001 {
            return field_ut;
        }

        let average_radius = (radii[0] + radii[1] + radii[2]) / 3.0;
        let center = [
            (self.max[0] + self.min[0]) * 0.5,
            (self.max[1] + self.min[1]) * 0.5,
            (self.max[2] + self.min[2]) * 0.5,
        ];

        [
            (field_ut[0] - center[0]) * average_radius / radii[0],
            (field_ut[1] - center[1]) * average_radius / radii[1],
            (field_ut[2] - center[2]) * average_radius / radii[2],
        ]
    }

    fn minimum_span(&self) -> f32 {
        if self.samples == 0 {
            return 0.0;
        }
        let x = self.max[0] - self.min[0];
        let y = self.max[1] - self.min[1];
        let z = self.max[2] - self.min[2];
        min_f32(x, min_f32(y, z))
    }
}

pub fn vector_length(value: [f32; 3]) -> f32 {
    sqrt_approx(value[0] * value[0] + value[1] * value[1] + value[2] * value[2])
}

fn min_f32(a: f32, b: f32) -> f32 {
    if a < b { a } else { b }
}

fn max_f32(a: f32, b: f32) -> f32 {
    if a > b { a } else { b }
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
