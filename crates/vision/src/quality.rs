//! A blur score: the variance of the Laplacian.
//!
//! The Laplacian is the sum of the second derivatives of the image. It is
//! large where the brightness changes quickly, at edges, and near zero on
//! flat areas and on soft gradients. A sharp image has strong edges, so its
//! Laplacian responses are spread widely and their variance is large. Blur
//! softens the edges, the responses shrink, and the variance drops. The
//! caller compares the score against a threshold found by trying a few
//! frames: the score depends on the camera, the resolution and the scene, so
//! there is no universal value.

use crate::image::GrayImage;

/// The widest row whose squared responses are summed in 32 bits: a
/// response is at most 1,020, its square 1,040,400, and 4,000 of those
/// are below `u32::MAX`.
pub const MAX_ROW: usize = 4000;

/// The variance of the 3x3 Laplacian of `image`: a blur score.
///
/// The Laplacian kernel is
///
/// ```text
/// 0  1  0
/// 1 -4  1
/// 0  1  0
/// ```
///
/// applied at every interior pixel; the 1-pixel border is left out because
/// the kernel would reach outside the image there. The result is the
/// population variance of the responses (the mean of the squares minus the
/// square of the mean). The sums are exact integers even for a whole
/// camera frame; only the final division is in floating point.
///
/// A row is summed in 32 bits and the rows in 64: a 64-bit addition or
/// multiplication per pixel costs several instructions on the board, and
/// a response is at most 1,020, so the squares of a row of up to
/// [`MAX_ROW`] pixels fit 32 bits.
///
/// Sharp images give large values, blurred ones small values. Images smaller
/// than 3x3 have no interior pixel and return 0.0.
pub fn laplacian_variance(image: &GrayImage<'_>) -> f32 {
    let width = image.width();
    let height = image.height();
    if width < 3 || height < 3 {
        return 0.0;
    }
    let mut sum = 0i64;
    let mut sum_of_squares = 0i64;
    for y in 1..height - 1 {
        let above = &image.row(y - 1)[1..width - 1];
        let row = image.row(y);
        let below = &image.row(y + 1)[1..width - 1];
        // The pixel, and its neighbours to the left and to the right.
        let left = &row[..width - 2];
        let centre = &row[1..width - 1];
        let right = &row[2..];
        let columns = above.iter().zip(below).zip(left).zip(centre).zip(right);
        if width - 2 <= MAX_ROW {
            let mut row_sum = 0i32;
            let mut row_squares = 0u32;
            for ((((&above, &below), &left), &centre), &right) in columns {
                let response =
                    i32::from(above) + i32::from(below) + i32::from(left) + i32::from(right)
                        - 4 * i32::from(centre);
                row_sum += response;
                row_squares += (response * response) as u32;
            }
            sum += i64::from(row_sum);
            sum_of_squares += i64::from(row_squares);
        } else {
            for ((((&above, &below), &left), &centre), &right) in columns {
                let response =
                    i64::from(above) + i64::from(below) + i64::from(left) + i64::from(right)
                        - 4 * i64::from(centre);
                sum += response;
                sum_of_squares += response * response;
            }
        }
    }
    // With `n` responses, the variance is `sum_of_squares / n - (sum / n)^2`
    // = `(n * sum_of_squares - sum * sum) / n^2`. The numerator is exact in
    // i64: for a 320x240 frame `n` is below 2^17 and each response is
    // between -1020 and 1020, so both products stay well below 2^63.
    let count = ((width - 2) * (height - 2)) as i64;
    let numerator = count * sum_of_squares - sum * sum;
    (numerator as f64 / (count as f64 * count as f64)) as f32
}
