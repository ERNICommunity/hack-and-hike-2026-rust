//! The Laplacian variance blur score on synthetic images.

use hack_and_hike_vision::image::GrayImage;
use hack_and_hike_vision::quality::{MAX_ROW, laplacian_variance};

/// A `size` x `size` checkerboard of black and white squares that are
/// `square` pixels wide.
fn checkerboard(size: usize, square: usize) -> Vec<u8> {
    let mut data = Vec::with_capacity(size * size);
    for y in 0..size {
        for x in 0..size {
            let white = (x / square + y / square).is_multiple_of(2);
            data.push(if white { 255 } else { 0 });
        }
    }
    data
}

/// A 3x3 box blur: every interior pixel becomes the rounded mean of itself
/// and its eight neighbours. The border is copied unchanged.
fn box_blur(data: &[u8], size: usize) -> Vec<u8> {
    let mut out = data.to_vec();
    for y in 1..size - 1 {
        for x in 1..size - 1 {
            let mut sum = 0u32;
            for dy in 0..3 {
                for dx in 0..3 {
                    sum += u32::from(data[(y + dy - 1) * size + (x + dx - 1)]);
                }
            }
            out[y * size + x] = ((sum + 4) / 9) as u8;
        }
    }
    out
}

#[test]
fn a_flat_image_scores_zero() {
    let data = vec![90u8; 20 * 12];
    let image = GrayImage::new(&data, 20, 12);
    assert_eq!(laplacian_variance(&image), 0.0);
}

#[test]
fn blur_lowers_the_score() {
    let sharp = checkerboard(32, 8);
    let blurred = box_blur(&sharp, 32);
    let sharp_score = laplacian_variance(&GrayImage::new(&sharp, 32, 32));
    let blurred_score = laplacian_variance(&GrayImage::new(&blurred, 32, 32));
    assert!(sharp_score > 0.0);
    assert!(
        sharp_score > blurred_score,
        "sharp {sharp_score} is not above blurred {blurred_score}"
    );
}

#[test]
fn images_without_an_interior_score_zero() {
    let data = [0u8, 255, 255, 0];
    assert_eq!(laplacian_variance(&GrayImage::new(&data, 2, 2)), 0.0);
    let row = [0u8, 255, 0, 255, 0];
    assert_eq!(laplacian_variance(&GrayImage::new(&row, 5, 1)), 0.0);
    assert_eq!(laplacian_variance(&GrayImage::new(&[], 0, 0)), 0.0);
}

/// The variance of the Laplacian with every sum in 64 bits, pixel by
/// pixel: what `laplacian_variance` must give, however it sums.
fn reference(data: &[u8], width: usize, height: usize) -> f32 {
    let at = |x: usize, y: usize| i64::from(data[y * width + x]);
    let (mut sum, mut squares, mut count) = (0i64, 0i64, 0i64);
    for y in 1..height - 1 {
        for x in 1..width - 1 {
            let response = at(x, y - 1) + at(x, y + 1) + at(x - 1, y) + at(x + 1, y) - 4 * at(x, y);
            sum += response;
            squares += response * response;
            count += 1;
        }
    }
    ((count * squares - sum * sum) as f64 / (count as f64 * count as f64)) as f32
}

#[test]
fn a_checkerboard_fills_the_32_bit_sums_to_the_limit() {
    // Every interior pixel of a one-pixel checkerboard responds with
    // +-1020, the most there is: a row of `MAX_ROW` of them is the
    // largest sum of squares the 32-bit path takes, 97 percent of its
    // range.
    for width in [MAX_ROW + 1, MAX_ROW + 2, MAX_ROW + 3] {
        let height = 4;
        let data: Vec<u8> = (0..width * height)
            .map(|index| {
                let (x, y) = (index % width, index / width);
                if (x + y) % 2 == 0 { 255 } else { 0 }
            })
            .collect();
        let score = laplacian_variance(&GrayImage::new(&data, width, height));
        assert_eq!(score, reference(&data, width, height), "width {width}");
        assert!(score > 1_000_000.0, "width {width}: {score}");
    }
}

#[test]
fn the_sums_are_exact_for_every_width() {
    // The strongest responses there are: single white pixels on black,
    // and single black pixels on white, give +-1020 at their centre.
    // Widths on both sides of the limit of the 32-bit row sums.
    for width in [5, 112, 320, MAX_ROW + 2, MAX_ROW + 3] {
        let height = 5;
        let mut data = vec![0u8; width * height];
        for (index, value) in data.iter_mut().enumerate() {
            let (x, y) = (index % width, index / width);
            let inverted = x >= width / 2;
            *value = if (x % 2 == 1 && y % 2 == 1) != inverted {
                255
            } else {
                0
            };
        }
        let score = laplacian_variance(&GrayImage::new(&data, width, height));
        assert_eq!(score, reference(&data, width, height), "width {width}");
        assert!(score > 100_000.0, "width {width}: {score}");
    }
}

#[test]
fn a_single_interior_pixel_has_no_spread() {
    // The centre is the only interior pixel. Its response is
    // 0 + 0 + 0 + 0 - 4 * 50 = -200, and the variance of one value is 0:
    // the mean of the squares (40000) minus the square of the mean (40000).
    let data = [0u8, 0, 0, 0, 50, 0, 0, 0, 0];
    assert_eq!(laplacian_variance(&GrayImage::new(&data, 3, 3)), 0.0);
}

#[test]
fn two_interior_pixels_give_the_exact_variance() {
    // A 4x3 image with one bright pixel at (1, 1). The interior pixels are
    // (1, 1) and (2, 1):
    //   response at (1, 1): 0 + 0 + 0 + 0 - 4 * 10 = -40
    //   response at (2, 1): 0 + 0 + 10 + 0 - 4 * 0 = 10
    // mean = -15, mean of squares = (1600 + 100) / 2 = 850,
    // variance = 850 - 225 = 625.
    let data = [0u8, 0, 0, 0, 0, 10, 0, 0, 0, 0, 0, 0];
    assert_eq!(laplacian_variance(&GrayImage::new(&data, 4, 3)), 625.0);
}
