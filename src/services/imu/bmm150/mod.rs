//! BMM150 magnetometer definitions, data decoding, and Bosch factory compensation.
//!
//! The BMI270 driver owns the auxiliary-bus transport used to reach this sensor.
//! Runtime hard/soft-iron calibration lives in the `calibration` submodule.
//!
//! The compensation equations are derived from Bosch Sensortec's BSD-3-Clause
//! BMM150 SensorAPI v2.0.0.

mod calibration;

pub use calibration::{Calibration, GOOD_FIELD_MAX_UT, GOOD_FIELD_MIN_UT, vector_length};

pub(super) const ADDRESS: u8 = 0x10;
pub(super) const CHIP_ID: u8 = 0x32;
pub(super) const REG_CHIP_ID: u8 = 0x40;
pub(super) const REG_DATA_X_LSB: u8 = 0x42;
pub(super) const REG_POWER_CONTROL: u8 = 0x4B;
pub(super) const REG_OP_MODE: u8 = 0x4C;
pub(super) const REG_REP_XY: u8 = 0x51;
pub(super) const REG_REP_Z: u8 = 0x52;
pub(super) const DIG_X1: u8 = 0x5D;
pub(super) const DIG_Z4_LSB: u8 = 0x62;
pub(super) const DIG_Z2_LSB: u8 = 0x68;

pub(super) const SOFT_RESET_AND_POWER: u8 = 0x83;
pub(super) const NORMAL_30HZ: u8 = 0x38;
pub(super) const REP_XY_REGULAR: u8 = 0x04;
pub(super) const REP_Z_REGULAR: u8 = 0x07;

const OVERFLOW_XY: i16 = -4096;
const OVERFLOW_Z: i16 = -16384;

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
