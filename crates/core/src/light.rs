//! Decoding of the LTR-553 proximity and ambient light sensor's data.
//!
//! The sensor reports two light counts and one proximity count. Channel 0 of
//! the ambient light sensor (ALS) sees visible and infrared light. Channel 1
//! sees mostly infrared light. Their ratio tells what kind of light it is:
//! sunlight has more infrared than an LED lamp. The datasheet gives one
//! linear formula for each range of that ratio. The formula turns the counts
//! into lux.

/// The two raw counts of one ambient light measurement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Channels {
    /// Visible and infrared light.
    pub ch0: u16,
    /// Mostly infrared light.
    pub ch1: u16,
}

/// One ambient light measurement: the counts and the gain they were taken
/// with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Light {
    /// The ambient light counts.
    pub channels: Channels,
    /// The gain the counts were measured with, as a factor: 1, 2, 4, 8, 48
    /// or 96. The sensor reports it with the data, so a gain change is
    /// never applied to counts taken at the old gain.
    pub gain: u8,
}

/// One decoded data block of the sensor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reading {
    /// The ambient light measurement; `None` while the sensor marks it
    /// invalid or reports a reserved gain. The proximity data does not
    /// depend on it.
    pub light: Option<Light>,
    /// Reflected light from the sensor's own infrared LED, 0 to
    /// [`PROXIMITY_MAX`]: 0 with nothing in front of the sensor, more the
    /// closer an object comes.
    pub proximity: u16,
    /// Whether the proximity measurement overflowed: something is very close.
    pub proximity_saturated: bool,
}

/// The largest proximity count: the measurement has 11 bits.
pub const PROXIMITY_MAX: u16 = 0x07FF;

/// Length of the data block starting at the first ALS data register.
pub const DATA_BLOCK_LEN: usize = 7;

/// The gain factor for a gain code, as written to `ALS_CONTR` bits 4:2 and
/// read back from `ALS_PS_STATUS` bits 6:4. Codes 4 and 5 are reserved and
/// give `None`.
pub fn gain_factor(code: u8) -> Option<u8> {
    match code {
        0 => Some(1),
        1 => Some(2),
        2 => Some(4),
        3 => Some(8),
        6 => Some(48),
        7 => Some(96),
        _ => None,
    }
}

/// Decode the seven registers starting at `ALS_DATA_CH1_0` (`0x88`).
///
/// The registers are: channel 1 low and high byte, channel 0 low and high
/// byte, the status register, proximity low and high byte.
///
/// [`Reading::light`] is `None` in two cases:
/// - The status says the light data is invalid. The sensor does this while
///   it is still measuring after a mode or gain change.
/// - The status reports a reserved gain code.
pub fn decode(block: [u8; DATA_BLOCK_LEN]) -> Reading {
    /// `ALS_PS_STATUS` bit 7: 1 means the ALS data is invalid.
    const ALS_DATA_INVALID: u8 = 0x80;
    /// `ALS_PS_STATUS` bits 6:4: the gain code of the data.
    const ALS_GAIN_SHIFT: u8 = 4;
    /// `PS_DATA_1` bit 7: the proximity measurement is saturated (at its
    /// maximum).
    const PROXIMITY_SATURATED: u8 = 0x80;

    let [
        ch1_low,
        ch1_high,
        ch0_low,
        ch0_high,
        status,
        ps_low,
        ps_high,
    ] = block;
    let light = if status & ALS_DATA_INVALID != 0 {
        None
    } else {
        gain_factor((status >> ALS_GAIN_SHIFT) & 0x07).map(|gain| Light {
            channels: Channels {
                ch0: u16::from_le_bytes([ch0_low, ch0_high]),
                ch1: u16::from_le_bytes([ch1_low, ch1_high]),
            },
            gain,
        })
    };
    Reading {
        light,
        proximity: u16::from_le_bytes([ps_low, ps_high]) & PROXIMITY_MAX,
        proximity_saturated: ps_high & PROXIMITY_SATURATED != 0,
    }
}

/// Illuminance in lux from the raw counts.
///
/// `gain` is the sensor's gain factor (1, 2, 4, 8, 48 or 96).
/// `integration_ms` is the integration time (measurement time) in
/// milliseconds, 50 to 400. Both must be nonzero, because the formula
/// divides by them.
///
/// The formula is from appendix A of the LTR-553ALS-WA datasheet. Light that
/// is almost only infrared (a ratio `ch1 / (ch0 + ch1)` of 0.85 or more)
/// gives 0 lux. The datasheet says so, because the sensor cannot measure
/// such light correctly.
///
/// # Panics
///
/// In debug builds, when `gain` or `integration_ms` is zero.
pub fn lux(channels: Channels, gain: u8, integration_ms: u16) -> f32 {
    debug_assert!(
        gain != 0 && integration_ms != 0,
        "gain and integration time are nonzero"
    );
    let ch0 = f32::from(channels.ch0);
    let ch1 = f32::from(channels.ch1);
    let total = ch0 + ch1;
    if total == 0.0 {
        return 0.0;
    }
    let ratio = ch1 / total;
    let raw = if ratio < 0.45 {
        1.7743 * ch0 + 1.1059 * ch1
    } else if ratio < 0.64 {
        4.2785 * ch0 - 1.9548 * ch1
    } else if ratio < 0.85 {
        0.5926 * ch0 + 0.1185 * ch1
    } else {
        0.0
    };
    let integration = f32::from(integration_ms) / 100.0;
    (raw / f32::from(gain) / integration).max(0.0)
}

/// Closeness in percent from a raw proximity count. The percentage is linear
/// in the distance: 0 at the edge of the range, 100 at the glass, 50 halfway.
///
/// The reflected light gets weaker with the square of the distance. So the
/// raw count changes very little while an object is far away, and very much
/// in the last centimetres. From that law, the distance is proportional to
/// `1 / sqrt(count)`.
///
/// `far_count` is the count at the edge of the range. `near_count` is the
/// count at the glass. A count at or below `far_count` gives 0, a count at
/// or above `near_count` gives 100.
pub fn closeness_percent(count: u16, far_count: u16, near_count: u16) -> u8 {
    if count <= far_count {
        return 0;
    }
    if count >= near_count {
        return 100;
    }
    // `distance` is the distance as a fraction of the range: 1.0 at
    // `far_count`. `at_glass` is that fraction at the glass. The sensor
    // cannot measure closer than the glass, so this is the smallest distance.
    let far = f32::from(far_count);
    let distance = libm::sqrtf(far / f32::from(count));
    let at_glass = libm::sqrtf(far / f32::from(near_count));
    let closeness = (1.0 - distance) / (1.0 - at_glass);
    libm::roundf(closeness.clamp(0.0, 1.0) * 100.0) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whether two lux values agree to a tenth of a percent of `b`, or to
    /// 0.001 when `b` is smaller than 1.
    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() <= 0.001 * b.abs().max(1.0)
    }

    #[test]
    fn decodes_channels_and_proximity() {
        let reading = decode([0x34, 0x12, 0x78, 0x56, 0x60, 0xFF, 0x07]);
        assert_eq!(
            reading.light,
            Some(Light {
                channels: Channels {
                    ch0: 0x5678,
                    ch1: 0x1234
                },
                gain: 48,
            })
        );
        assert_eq!(reading.proximity, PROXIMITY_MAX);
        assert!(!reading.proximity_saturated);
    }

    #[test]
    fn saturation_flag_is_not_part_of_the_count() {
        let reading = decode([0, 0, 0, 0, 0x00, 0x01, 0x80]);
        assert_eq!(reading.proximity, 1);
        assert!(reading.proximity_saturated);
    }

    #[test]
    fn invalid_light_data_keeps_the_proximity() {
        let reading = decode([1, 0, 1, 0, 0x80, 0x34, 0x02]);
        assert_eq!(reading.light, None);
        assert_eq!(reading.proximity, 0x234);
    }

    #[test]
    fn reserved_gain_codes_are_rejected() {
        assert_eq!(decode([1, 0, 1, 0, 0x40, 0, 0]).light, None);
        assert_eq!(decode([1, 0, 1, 0, 0x50, 0, 0]).light, None);
        assert_eq!(decode([1, 0, 1, 0, 0x70, 0, 0]).light.unwrap().gain, 96);
    }

    #[test]
    fn darkness_is_zero_lux() {
        assert_eq!(lux(Channels { ch0: 0, ch1: 0 }, 1, 100), 0.0);
    }

    #[test]
    fn each_ratio_range_uses_its_own_formula() {
        // ratio 0.2: first range
        assert!(close(
            lux(Channels { ch0: 800, ch1: 200 }, 1, 100),
            1.7743 * 800.0 + 1.1059 * 200.0
        ));
        // ratio 0.5: second range
        assert!(close(
            lux(Channels { ch0: 500, ch1: 500 }, 1, 100),
            4.2785 * 500.0 - 1.9548 * 500.0
        ));
        // ratio 0.75: third range
        assert!(close(
            lux(Channels { ch0: 250, ch1: 750 }, 1, 100),
            0.5926 * 250.0 + 0.1185 * 750.0
        ));
        // ratio 0.9: all infrared
        assert_eq!(lux(Channels { ch0: 100, ch1: 900 }, 1, 100), 0.0);
    }

    #[test]
    fn closeness_is_clamped_at_both_ends() {
        assert_eq!(closeness_percent(0, 16, 2047), 0);
        assert_eq!(closeness_percent(16, 16, 2047), 0);
        assert_eq!(closeness_percent(2047, 16, 2047), 100);
        assert_eq!(closeness_percent(u16::MAX, 16, 2047), 100);
    }

    #[test]
    fn closeness_is_linear_in_distance() {
        // With the glass at almost zero distance: four times the far count
        // is half the distance, and twenty-five times the far count is a
        // fifth of the distance.
        assert!((50..=51).contains(&closeness_percent(64, 16, u16::MAX)));
        assert!((80..=81).contains(&closeness_percent(400, 16, u16::MAX)));
        // With a realistic near count, the scale is stretched so that the
        // glass is at 100.
        let quarter = closeness_percent(64, 16, 2047);
        assert!((50..=56).contains(&quarter), "{quarter}");
    }

    #[test]
    fn closeness_never_decreases_with_the_count() {
        let mut previous = 0;
        for count in 0..=2047 {
            let closeness = closeness_percent(count, 16, 2047);
            assert!(closeness >= previous, "count {count}");
            previous = closeness;
        }
    }

    #[test]
    fn gain_and_integration_time_scale_the_result() {
        let channels = Channels { ch0: 800, ch1: 200 };
        let base = lux(channels, 1, 100);
        assert!(close(lux(channels, 8, 100), base / 8.0));
        assert!(close(lux(channels, 1, 400), base / 4.0));
        assert!(close(lux(channels, 1, 50), base * 2.0));
    }
}
