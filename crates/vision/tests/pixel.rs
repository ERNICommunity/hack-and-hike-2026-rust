//! Pixel conversions checked against an independent reference over every
//! one of the 65536 RGB565 values.

use hack_and_hike_vision::pixel::{
    BYTES_PER_PIXEL, rgb565_be_to_gray, rgb565_be_to_rgb888, rgb888_to_gray,
};

/// A lookup table from every `bits`-bit value to 8 bits, filled by repeating
/// the bit pattern until it is at least 8 bits long and keeping the top 8.
/// This is bit replication spelled differently from the library.
fn replication_table(bits: u32) -> Vec<u8> {
    (0..1u32 << bits)
        .map(|value| {
            let mut repeated = 0u32;
            let mut filled = 0;
            while filled < 8 {
                repeated = (repeated << bits) | value;
                filled += bits;
            }
            (repeated >> (filled - 8)) as u8
        })
        .collect()
}

/// The reference conversion: the fields are cut out of the 16-bit value
/// with masks and shifts, then expanded by table lookup.
fn reference_rgb888(bytes: [u8; BYTES_PER_PIXEL]) -> [u8; 3] {
    let table5 = replication_table(5);
    let table6 = replication_table(6);
    let value = u16::from_be_bytes(bytes);
    let red = usize::from((value >> 11) & 0x1F);
    let green = usize::from((value >> 5) & 0x3F);
    let blue = usize::from(value & 0x1F);
    [table5[red], table6[green], table5[blue]]
}

/// The reference gray formula, BT.601 weights scaled to 256 with rounding.
fn reference_gray([red, green, blue]: [u8; 3]) -> u8 {
    let weighted = 77 * u32::from(red) + 150 * u32::from(green) + 29 * u32::from(blue);
    ((weighted + 128) >> 8) as u8
}

#[test]
fn every_rgb565_value_matches_the_reference() {
    for value in 0..=u16::MAX {
        let bytes = value.to_be_bytes();
        let expected = reference_rgb888(bytes);
        assert_eq!(rgb565_be_to_rgb888(bytes), expected, "value {value:#06x}");
        assert_eq!(rgb888_to_gray(expected), reference_gray(expected));
        assert_eq!(
            rgb565_be_to_gray(bytes),
            reference_gray(expected),
            "value {value:#06x}"
        );
    }
}

#[test]
fn endpoints_and_pure_colours() {
    assert_eq!(rgb565_be_to_rgb888([0x00, 0x00]), [0, 0, 0]);
    assert_eq!(rgb565_be_to_rgb888([0xFF, 0xFF]), [255, 255, 255]);
    assert_eq!(rgb565_be_to_rgb888([0xF8, 0x00]), [255, 0, 0]);
    assert_eq!(rgb565_be_to_rgb888([0x07, 0xE0]), [0, 255, 0]);
    assert_eq!(rgb565_be_to_rgb888([0x00, 0x1F]), [0, 0, 255]);
    assert_eq!(rgb565_be_to_gray([0x00, 0x00]), 0);
    assert_eq!(rgb565_be_to_gray([0xFF, 0xFF]), 255);
}

#[test]
fn the_first_byte_holds_red() {
    // Read little-endian by mistake, 0xF8 0x00 would be 0x00F8: full blue
    // and a bit of green.
    let [red, green, blue] = rgb565_be_to_rgb888([0xF8, 0x00]);
    assert_eq!(red, 255);
    assert_eq!(green, 0);
    assert_eq!(blue, 0);
}

#[test]
fn gray_weights_sum_to_full_scale() {
    assert_eq!(rgb888_to_gray([255, 255, 255]), 255);
    assert_eq!(rgb888_to_gray([0, 0, 0]), 0);
    // Green weighs the most, blue the least.
    assert!(rgb888_to_gray([0, 255, 0]) > rgb888_to_gray([255, 0, 0]));
    assert!(rgb888_to_gray([255, 0, 0]) > rgb888_to_gray([0, 0, 255]));
}
