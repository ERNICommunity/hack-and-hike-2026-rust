//! BMM150 magnetometer data decoding and Bosch factory compensation.
//!
//! The firmware's BMI270 driver reads and writes the registers of this
//! sensor. It passes the raw data frame and the trim bytes to this module.
//! Trim values are correction values that the factory stores in each chip.
//!
//! The `calibration` submodule learns the hard-iron and soft-iron distortion
//! at run time (see [`Calibration`]).
//!
//! The compensation equations come from Bosch Sensortec's BMM150 `SensorAPI`
//! v2.0.0 (BSD-3-Clause license).

mod calibration;

pub use calibration::Calibration;

/// Raw 13-bit X or Y value (after the shift in [`compensate`]) that the
/// sensor reports when the axis overflowed.
const OVERFLOW_XY: i16 = -4096;
/// Raw 15-bit Z value (after the shift in [`compensate`]) that the sensor
/// reports when the axis overflowed.
const OVERFLOW_Z: i16 = -16384;

/// Factory trim values, read from the sensor once at start-up.
///
/// Multi-byte values are little-endian: the low byte is in the first
/// register.
#[derive(Clone, Copy, Debug)]
pub struct Trim {
    /// X offset trim, register `0x5D`. Added to the scaled X reading.
    dig_x1: i8,
    /// Y offset trim, register `0x5E`. Added to the scaled Y reading.
    dig_y1: i8,
    /// X sensitivity trim, register `0x64`.
    dig_x2: i8,
    /// Y sensitivity trim, register `0x65`.
    dig_y2: i8,
    /// Z sensitivity trim that is multiplied by the Hall resistance,
    /// registers `0x6A` and `0x6B`.
    dig_z1: u16,
    /// Z sensitivity trim, registers `0x68` and `0x69`. Zero means the trim
    /// data is invalid. Bosch's compensation checks this, because the Z
    /// divisor contains it.
    dig_z2: i16,
    /// Z trim for the difference between the Hall resistance and
    /// `dig_xyz1`, registers `0x6E` and `0x6F`.
    dig_z3: i16,
    /// Z offset trim, registers `0x62` and `0x63`. Subtracted from the raw Z
    /// reading.
    dig_z4: i16,
    /// Linear coefficient of the Hall resistance correction for X and Y,
    /// register `0x71`.
    dig_xy1: u8,
    /// Quadratic coefficient of the Hall resistance correction for X and Y,
    /// register `0x70`.
    dig_xy2: i8,
    /// Reference Hall resistance that the measured Hall resistance is
    /// compared with, registers `0x6C` and `0x6D`. It has 15 bits: the top
    /// bit of `0x6D` is removed. Zero means the trim data is invalid.
    dig_xyz1: u16,
}

impl Trim {
    /// Build the trim values from the three register blocks that Bosch
    /// documents: `0x5D` to `0x5E`, `0x62` to `0x65`, and `0x68` to `0x71`.
    pub fn from_registers(x1_y1: [u8; 2], z4_x2_y2: [u8; 4], z2_to_xy1: [u8; 10]) -> Self {
        Self {
            dig_x1: i8::from_ne_bytes([x1_y1[0]]),
            dig_y1: i8::from_ne_bytes([x1_y1[1]]),
            dig_x2: i8::from_ne_bytes([z4_x2_y2[2]]),
            dig_y2: i8::from_ne_bytes([z4_x2_y2[3]]),
            dig_z1: u16::from_le_bytes([z2_to_xy1[2], z2_to_xy1[3]]),
            dig_z2: i16::from_le_bytes([z2_to_xy1[0], z2_to_xy1[1]]),
            dig_z3: i16::from_le_bytes([z2_to_xy1[6], z2_to_xy1[7]]),
            dig_z4: i16::from_le_bytes([z4_x2_y2[0], z4_x2_y2[1]]),
            dig_xy1: z2_to_xy1[9],
            dig_xy2: i8::from_ne_bytes([z2_to_xy1[8]]),
            dig_xyz1: u16::from_le_bytes([z2_to_xy1[4], z2_to_xy1[5] & 0x7f]),
        }
    }
}

/// One compensated magnetometer reading.
#[derive(Clone, Copy, Debug)]
pub struct Sample {
    /// Compensated field in uT (microtesla), in the magnetometer's own axes.
    pub field_ut: [f32; 3],
    /// Whether the sensor flagged this frame as a fresh measurement.
    pub data_ready: bool,
}

/// Decode the BMM150's 8-byte data frame and apply Bosch's factory
/// compensation.
///
/// The frame holds X, Y, Z and RHALL (the resistance of the Hall sensor),
/// low byte first. Returns `None` when an axis overflowed, when RHALL is
/// zero, or when the trim data is invalid.
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

    let x = compensate_xy(raw_x, rhall, trim.dig_x1, trim.dig_x2, trim);
    let y = compensate_xy(raw_y, rhall, trim.dig_y1, trim.dig_y2, trim);
    let z = compensate_z(raw_z, rhall, trim)?;

    Some(Sample {
        field_ut: [x, y, z],
        data_ready,
    })
}

/// Bosch's floating-point compensation of one X or Y reading, in uT.
/// `dig_1` and `dig_2` are the offset and sensitivity trims of that axis.
/// `rhall` is the Hall resistance from the same data frame.
fn compensate_xy(raw: i16, rhall: u16, dig_1: i8, dig_2: i8, trim: Trim) -> f32 {
    let x0 = f32::from(trim.dig_xyz1) * 16384.0 / f32::from(rhall);
    let ratio = x0 - 16384.0;
    let x1 = f32::from(trim.dig_xy2) * (ratio * ratio / 268_435_456.0);
    let x2 = x1 + ratio * f32::from(trim.dig_xy1) / 16384.0;
    let x3 = f32::from(dig_2) + 160.0;
    let x4 = f32::from(raw) * ((x2 + 256.0) * x3);
    ((x4 / 8192.0) + f32::from(dig_1) * 8.0) / 16.0
}

/// Bosch's floating-point compensation of the Z reading, in uT.
/// `None` when the trim values and `rhall` make the divisor (almost) zero.
fn compensate_z(raw: i16, rhall: u16, trim: Trim) -> Option<f32> {
    let z0 = f32::from(raw) - f32::from(trim.dig_z4);
    let z1 = f32::from(rhall) - f32::from(trim.dig_xyz1);
    let z2 = f32::from(trim.dig_z3) * z1;
    let z3 = f32::from(trim.dig_z1) * f32::from(rhall) / 32768.0;
    let z4 = f32::from(trim.dig_z2) + z3;
    if z4.abs() < 0.0001 {
        return None;
    }
    let z5 = z0 * 131072.0 - z2;
    Some((z5 / (z4 * 4.0)) / 16.0)
}
