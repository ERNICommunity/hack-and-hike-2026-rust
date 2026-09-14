//! Decoding of the FT6336 touch controller's report.

/// Position of a finger in the controller's own coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RawTouch {
    /// Column, 0 to 319.
    pub x: u16,
    /// Row, 0 to 239.
    pub y: u16,
}

/// Decode the five bytes starting at register `0x02`: the touch count and
/// the first point. `None` when no finger is down.
///
/// The low nibble of the first byte is the number of fingers; the high
/// nibbles of the coordinate bytes carry event flags and are ignored.
pub fn decode_report(report: [u8; 5]) -> Option<RawTouch> {
    if report[0] & 0x0F == 0 {
        return None;
    }
    Some(RawTouch {
        x: (u16::from(report[1] & 0x0F) << 8) | u16::from(report[2]),
        y: (u16::from(report[3] & 0x0F) << 8) | u16::from(report[4]),
    })
}

/// Map a point of a `width` x `height` panel onto the same panel mounted
/// upside down.
pub const fn rotate_180(x: u16, y: u16, width: u16, height: u16) -> (u16, u16) {
    (width - 1 - x, height - 1 - y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_the_first_point() {
        let report = [0x01, 0x81, 0x2C, 0x00, 0xE0];
        assert_eq!(decode_report(report), Some(RawTouch { x: 300, y: 224 }));
    }

    #[test]
    fn no_fingers_means_no_point() {
        assert_eq!(decode_report([0x00, 0x81, 0x2C, 0x00, 0xE0]), None);
    }

    #[test]
    fn rotation_flips_both_axes() {
        assert_eq!(rotate_180(0, 0, 320, 240), (319, 239));
        assert_eq!(rotate_180(319, 239, 320, 240), (0, 0));
    }
}
