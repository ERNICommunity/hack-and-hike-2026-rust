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
/// square of the mean). The sums are `i64` and stay exact even for a whole
/// camera frame; only the final division is in floating point.
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
        let above = image.row(y - 1);
        let row = image.row(y);
        let below = image.row(y + 1);
        for x in 1..width - 1 {
            let response = i64::from(above[x])
                + i64::from(below[x])
                + i64::from(row[x - 1])
                + i64::from(row[x + 1])
                - 4 * i64::from(row[x]);
            sum += response;
            sum_of_squares += response * response;
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
