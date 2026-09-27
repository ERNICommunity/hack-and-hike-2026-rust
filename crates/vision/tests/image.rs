//! Image views and box downscaling on small hand-made frames.

use hack_and_hike_vision::image::{
    GrayImageMut, Image, ImageMut, Rgb565Frame, Rgb565Source, RgbImageMut, downscale_to_gray,
    downscale_to_rgb,
};

/// Big-endian RGB565 bytes with a 5-bit red value, no green and no blue.
fn red_pixel(red5: u8) -> [u8; 2] {
    [red5 << 3, 0x00]
}

/// A frame of `width` x `height` pixels where every pixel is `pixel`.
fn solid_frame(pixel: [u8; 2], width: usize, height: usize) -> Vec<u8> {
    pixel.repeat(width * height)
}

#[test]
fn downscale_to_gray_averages_blocks_of_a_gradient() {
    // A 16x8 frame. Column x has the 5-bit red value 2x, so the columns run
    // from black to nearly pure red. Every row is the same.
    //
    // Red expands to 8 bits by bit replication, and gray is
    // (77 * red + 128) >> 8 with green and blue zero:
    //
    //   x:    0  1   2   3   4   5   6    7    8    9   10   11   12   13   14   15
    //   red5: 0  2   4   6   8  10  12   14   16   18   20   22   24   26   28   30
    //   red8: 0 16  33  49  66  82  99  115  132  148  165  181  198  214  231  247
    //   gray: 0  5  10  15  20  25  30   35   40   45   50   54   60   64   69   74
    //
    // A 4x4 block covers four columns and four equal rows, so its sum is
    // four times the sum of the four column grays, and the mean is
    // (sum + 8) / 16:
    //
    //   columns 0..4:   4 * (0 + 5 + 10 + 15)   = 120 -> (120 + 8) / 16 = 8
    //   columns 4..8:   4 * (20 + 25 + 30 + 35) = 440 -> 448 / 16 = 28
    //   columns 8..12:  4 * (40 + 45 + 50 + 54) = 756 -> 764 / 16 = 47
    //   columns 12..16: 4 * (60 + 64 + 69 + 74) = 1068 -> 1076 / 16 = 67
    let mut frame = Vec::new();
    for _y in 0..8 {
        for x in 0..16u8 {
            frame.extend_from_slice(&red_pixel(2 * x));
        }
    }
    let src = Rgb565Frame::new(&frame, 16, 8);
    let mut out = [0u8; 4 * 2];
    let mut dst = GrayImageMut::new(&mut out, 4, 2);
    downscale_to_gray(&src, 4, &mut dst);
    assert_eq!(out, [8, 28, 47, 67, 8, 28, 47, 67]);
}

#[test]
fn downscale_to_gray_rounds_half_up() {
    // Black, white / white, black: the grays are 0, 255, 255, 0 with the
    // mean 127.5, which rounds to 128.
    let frame = [0x00, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0x00, 0x00];
    let src = Rgb565Frame::new(&frame, 2, 2);
    let mut out = [0u8; 1];
    let mut dst = GrayImageMut::new(&mut out, 1, 1);
    downscale_to_gray(&src, 2, &mut dst);
    assert_eq!(out, [128]);
}

#[test]
fn downscale_to_rgb_keeps_pure_red() {
    let frame = solid_frame([0xF8, 0x00], 6, 4);
    let src = Rgb565Frame::new(&frame, 6, 4);
    let mut out = [0u8; 3 * 2 * 3];
    let mut dst = RgbImageMut::new(&mut out, 3, 2);
    downscale_to_rgb(&src, 2, &mut dst);
    for y in 0..2 {
        for x in 0..3 {
            assert_eq!(dst.pixel(x, y), [255, 0, 0], "pixel ({x}, {y})");
        }
    }
}

#[test]
fn downscale_by_one_is_a_conversion() {
    let frame = [0xF8, 0x00, 0x07, 0xE0, 0x00, 0x1F, 0xFF, 0xFF];
    let src = Rgb565Frame::new(&frame, 4, 1);
    let mut out = [0u8; 4 * 3];
    let mut dst = RgbImageMut::new(&mut out, 4, 1);
    downscale_to_rgb(&src, 1, &mut dst);
    assert_eq!(out, [255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255]);
}

#[test]
#[should_panic(expected = "multiple of the factor")]
fn downscale_rejects_a_source_that_is_not_a_multiple() {
    let frame = solid_frame([0, 0], 6, 4);
    let src = Rgb565Frame::new(&frame, 6, 4);
    let mut out = [0u8; 1];
    let mut dst = GrayImageMut::new(&mut out, 1, 1);
    downscale_to_gray(&src, 4, &mut dst);
}

#[test]
#[should_panic(expected = "destination size")]
fn downscale_rejects_a_destination_of_the_wrong_size() {
    let frame = solid_frame([0, 0], 8, 4);
    let src = Rgb565Frame::new(&frame, 8, 4);
    let mut out = [0u8; 4 * 4];
    let mut dst = GrayImageMut::new(&mut out, 4, 4);
    downscale_to_gray(&src, 2, &mut dst);
}

#[test]
#[should_panic(expected = "at least 1")]
fn downscale_rejects_a_factor_of_zero() {
    let frame = solid_frame([0, 0], 2, 2);
    let src = Rgb565Frame::new(&frame, 2, 2);
    let mut out = [0u8; 4];
    let mut dst = GrayImageMut::new(&mut out, 2, 2);
    downscale_to_gray(&src, 0, &mut dst);
}

#[test]
#[should_panic(expected = "frame buffer length")]
fn frame_rejects_a_buffer_of_the_wrong_length() {
    let frame = [0u8; 7];
    let _ = Rgb565Frame::new(&frame, 2, 2);
}

#[test]
#[should_panic(expected = "image buffer length")]
fn image_rejects_a_buffer_of_the_wrong_length() {
    let data = [0u8; 5];
    let _ = Image::<3>::new(&data, 2, 1);
}

#[test]
#[should_panic(expected = "image buffer length")]
fn image_mut_rejects_a_buffer_of_the_wrong_length() {
    let mut data = [0u8; 3];
    let _ = ImageMut::<1>::new(&mut data, 2, 2);
}

#[test]
fn frame_rows_are_two_bytes_per_pixel() {
    let frame: Vec<u8> = (0..12).collect();
    let src = Rgb565Frame::new(&frame, 3, 2);
    assert_eq!(src.width(), 3);
    assert_eq!(src.height(), 2);
    assert_eq!(src.row(0), [0, 1, 2, 3, 4, 5]);
    assert_eq!(src.row(1), [6, 7, 8, 9, 10, 11]);
}

#[test]
fn image_indexes_pixels_and_rows() {
    // A 3x2 RGB image whose bytes count up.
    let data: Vec<u8> = (0..18).collect();
    let image = Image::<3>::new(&data, 3, 2);
    assert_eq!(image.width(), 3);
    assert_eq!(image.height(), 2);
    assert_eq!(image.row(0), &data[..9]);
    assert_eq!(image.row(1), &data[9..]);
    assert_eq!(image.pixel(0, 0), [0, 1, 2]);
    assert_eq!(image.pixel(2, 0), [6, 7, 8]);
    assert_eq!(image.pixel(1, 1), [12, 13, 14]);
    assert_eq!(image.data(), &data[..]);
}

#[test]
#[should_panic(expected = "inside the width")]
fn image_rejects_x_past_the_row() {
    let data = [0u8; 6];
    let image = Image::<1>::new(&data, 3, 2);
    let _ = image.pixel(3, 0);
}

#[test]
fn image_mut_writes_pixels_and_rows() {
    let mut data = [0u8; 8];
    let mut image = ImageMut::<2>::new(&mut data, 2, 2);
    image.set_pixel(1, 0, [10, 11]);
    image.row_mut(1).copy_from_slice(&[20, 21, 22, 23]);
    assert_eq!(image.pixel(1, 0), [10, 11]);
    assert_eq!(image.row(1), [20, 21, 22, 23]);
    let view = image.as_image();
    assert_eq!(view.pixel(0, 1), [20, 21]);
    assert_eq!(view.pixel(1, 1), [22, 23]);
    assert_eq!(image.data(), [0, 0, 10, 11, 20, 21, 22, 23]);
}
