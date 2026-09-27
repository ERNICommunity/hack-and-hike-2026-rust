//! One pixel at a time: RGB565 to RGB888 and to gray.
//!
//! The camera sends RGB565: 16 bits per pixel with 5 bits of red, 6 bits of
//! green and 5 bits of blue. Green gets the extra bit because the eye is most
//! sensitive to green. The two bytes of a pixel arrive big-endian: the first
//! byte holds red and the top 3 bits of green, the second byte holds the low
//! 3 bits of green and blue.
//!
//! ```text
//! first byte:  R4 R3 R2 R1 R0 G5 G4 G3
//! second byte: G2 G1 G0 B4 B3 B2 B1 B0
//! ```
//!
//! The neural networks and the blur score work on 8-bit channels, so the
//! functions here expand the narrow channels to 8 bits and mix them into one
//! gray value.

/// Number of bytes of one RGB565 pixel.
pub const BYTES_PER_PIXEL: usize = 2;

/// Expand a 5-bit value (0 to 31) to 8 bits (0 to 255).
///
/// A plain shift left by 3 leaves the low 3 bits zero, so 31 would become
/// 248 and pure white would not be white. Copying the top 3 bits of the value
/// into those low bits fixes this: 0 stays 0, 31 becomes 255, and the values
/// in between are spread evenly. This is called bit replication.
fn expand_5_bits(value: u8) -> u8 {
    (value << 3) | (value >> 2)
}

/// Expand a 6-bit value (0 to 63) to 8 bits (0 to 255), by bit replication
/// like [`expand_5_bits`]: the top 2 bits fill the low 2 bits.
fn expand_6_bits(value: u8) -> u8 {
    (value << 2) | (value >> 4)
}

/// Convert one big-endian RGB565 pixel to 8-bit red, green and blue.
pub fn rgb565_be_to_rgb888(bytes: [u8; BYTES_PER_PIXEL]) -> [u8; 3] {
    let [first, second] = bytes;
    let red = first >> 3;
    let green = ((first & 0x07) << 3) | (second >> 5);
    let blue = second & 0x1F;
    [
        expand_5_bits(red),
        expand_6_bits(green),
        expand_5_bits(blue),
    ]
}

/// Convert 8-bit red, green and blue to one 8-bit gray value.
///
/// The weights are the BT.601 luma weights (0.299, 0.587, 0.114) scaled to a
/// sum of 256, so that the division is a shift: 77 + 150 + 29 = 256. The
/// `+ 128` rounds to the nearest value instead of always rounding down. The
/// arithmetic is in `u32`; the largest intermediate value is
/// `256 * 255 + 128`, which is far below the limit.
pub fn rgb888_to_gray(rgb: [u8; 3]) -> u8 {
    let [red, green, blue] = rgb;
    let weighted = 77 * u32::from(red) + 150 * u32::from(green) + 29 * u32::from(blue);
    ((weighted + 128) >> 8) as u8
}

/// Convert one big-endian RGB565 pixel straight to 8-bit gray.
pub fn rgb565_be_to_gray(bytes: [u8; BYTES_PER_PIXEL]) -> u8 {
    rgb888_to_gray(rgb565_be_to_rgb888(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replication_keeps_the_endpoints() {
        assert_eq!(expand_5_bits(0), 0);
        assert_eq!(expand_5_bits(31), 255);
        assert_eq!(expand_6_bits(0), 0);
        assert_eq!(expand_6_bits(63), 255);
    }

    #[test]
    fn replication_never_decreases() {
        for value in 1..32 {
            assert!(expand_5_bits(value) > expand_5_bits(value - 1));
        }
        for value in 1..64 {
            assert!(expand_6_bits(value) > expand_6_bits(value - 1));
        }
    }
}
