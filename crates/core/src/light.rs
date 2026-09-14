//! Decoding of the LTR-553 proximity and ambient light sensor's data.
//!
//! The sensor reports two light counts and one proximity count. Channel 0 of
//! the light sensor sees visible and infrared light, channel 1 mostly
//! infrared. Their ratio tells what kind of light it is (sunlight carries
//! more infrared than an LED lamp), and the datasheet gives one linear
//! formula per range of that ratio to turn the counts into lux.

/// The two raw counts of one ambient light measurement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Channels {
    /// Visible and infrared light.
    pub ch0: u16,
    /// Mostly infrared light.
    pub ch1: u16,
}

/// One decoded data block of the sensor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reading {
    /// The ambient light counts.
    pub channels: Channels,
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

/// Decode the seven registers starting at `ALS_DATA_CH1_0` (`0x88`): channel
/// 1 low and high, channel 0 low and high, the status register, proximity
/// low and high. `None` when the status says the light data is invalid, which
/// the sensor signals while it is still integrating after a mode change.
pub fn decode(block: [u8; DATA_BLOCK_LEN]) -> Option<Reading> {
    /// ALS_PS_STATUS bit 7: 1 = the ALS data is invalid.
    const ALS_DATA_INVALID: u8 = 0x80;
    /// PS_DATA_1 bit 7: the proximity measurement saturated.
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
    if status & ALS_DATA_INVALID != 0 {
        return None;
    }
    Some(Reading {
        channels: Channels {
            ch0: u16::from_le_bytes([ch0_low, ch0_high]),
            ch1: u16::from_le_bytes([ch1_low, ch1_high]),
        },
        proximity: u16::from_le_bytes([ps_low, ps_high]) & PROXIMITY_MAX,
        proximity_saturated: ps_high & PROXIMITY_SATURATED != 0,
    })
}

/// Illuminance in lux from the raw counts, for the sensor's `gain` factor
/// (1, 2, 4, 8, 48 or 96) and integration time in milliseconds (50 to 400).
///
/// The formula is appendix A of the LTR-553ALS-WA datasheet. Light that is
/// almost entirely infrared (a ratio of 0.85 or more) counts as 0 lux, as the
/// datasheet prescribes: the sensor cannot judge it.
pub fn lux(channels: Channels, gain: u8, integration_ms: u16) -> f32 {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Whether two lux values agree to a tenth of a percent.
    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() <= 0.001 * b.abs().max(1.0)
    }

    #[test]
    fn decodes_channels_and_proximity() {
        let reading = decode([0x34, 0x12, 0x78, 0x56, 0x00, 0xFF, 0x07]).unwrap();
        assert_eq!(
            reading.channels,
            Channels {
                ch0: 0x5678,
                ch1: 0x1234
            }
        );
        assert_eq!(reading.proximity, PROXIMITY_MAX);
        assert!(!reading.proximity_saturated);
    }

    #[test]
    fn saturation_flag_is_not_part_of_the_count() {
        let reading = decode([0, 0, 0, 0, 0x00, 0x01, 0x80]).unwrap();
        assert_eq!(reading.proximity, 1);
        assert!(reading.proximity_saturated);
    }

    #[test]
    fn invalid_light_data_is_rejected() {
        assert_eq!(decode([1, 0, 1, 0, 0x80, 0, 0]), None);
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
    fn gain_and_integration_time_scale_the_result() {
        let channels = Channels { ch0: 800, ch1: 200 };
        let base = lux(channels, 1, 100);
        assert!(close(lux(channels, 8, 100), base / 8.0));
        assert!(close(lux(channels, 1, 400), base / 4.0));
        assert!(close(lux(channels, 1, 50), base * 2.0));
    }
}
