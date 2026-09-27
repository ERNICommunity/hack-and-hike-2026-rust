//! Similarity transform, landmark fitting and bilinear warp: face alignment.
//!
//! A face recognizer wants every face in the same pose: eyes on one line,
//! the same size, in the same place of a small crop. The face detector finds
//! five landmarks (eyes, nose tip, mouth corners). [`fit`] computes the
//! transform that moves those five points as close as possible onto the
//! standard positions of [`ARCFACE_TEMPLATE_112`], and [`warp`] draws the
//! aligned crop by sampling the source image through that transform.
//!
//! The transform is a similarity: rotation, uniform scale and translation.
//! It keeps the face's shape, unlike a general affine transform which could
//! shear it.
//!
//! [`warp`] runs backwards, as image warps always do: for every destination
//! pixel it asks where that pixel comes from in the source. So the transform
//! it takes maps destination coordinates to source coordinates. [`fit`] gives
//! the source-to-destination direction; use [`Similarity::inverse`] between
//! the two.

use crate::image::{Image, ImageMut};

/// A similarity transform: rotation, uniform scale and translation.
///
/// It maps `(x, y)` to `(a*x - b*y + tx, b*x + a*y + ty)`. With `a = s*cos(r)`
/// and `b = s*sin(r)` this rotates by `r` and scales by `s`. Written as
/// complex numbers, the linear part is a multiplication by `a + i*b`, which
/// makes composition and inversion short.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Similarity {
    /// `scale * cos(rotation)`.
    pub a: f32,
    /// `scale * sin(rotation)`.
    pub b: f32,
    /// Translation along x, applied after the rotation and scale.
    pub tx: f32,
    /// Translation along y, applied after the rotation and scale.
    pub ty: f32,
}

impl Similarity {
    /// The transform that changes nothing.
    pub const IDENTITY: Self = Self {
        a: 1.0,
        b: 0.0,
        tx: 0.0,
        ty: 0.0,
    };

    /// Transform one point.
    pub fn apply(&self, point: [f32; 2]) -> [f32; 2] {
        let [x, y] = point;
        [
            self.a * x - self.b * y + self.tx,
            self.b * x + self.a * y + self.ty,
        ]
    }

    /// The transform that undoes this one, or `None` when this one squashes
    /// everything to a point (scale zero) or has non-finite coefficients.
    ///
    /// The linear part `a + i*b` has the inverse `(a - i*b) / (a*a + b*b)`.
    /// The inverse translation moves the image of the origin back.
    pub fn inverse(&self) -> Option<Similarity> {
        let squared_scale = self.a * self.a + self.b * self.b;
        if !squared_scale.is_finite() || squared_scale < 1e-12 {
            return None;
        }
        let a = self.a / squared_scale;
        let b = -self.b / squared_scale;
        Some(Self {
            a,
            b,
            tx: -(a * self.tx - b * self.ty),
            ty: -(b * self.tx + a * self.ty),
        })
    }

    /// The transform that applies `self` first and then `next`.
    ///
    /// The linear parts multiply like complex numbers. The translation of
    /// `self` goes through `next` like any other point.
    pub fn then(&self, next: &Similarity) -> Similarity {
        let [tx, ty] = next.apply([self.tx, self.ty]);
        Self {
            a: next.a * self.a - next.b * self.b,
            b: next.b * self.a + next.a * self.b,
            tx,
            ty,
        }
    }
}

/// The least-squares similarity that maps each `src[i]` onto `dst[i]`.
///
/// `None` when the slices differ in length, hold fewer than 2 points, all
/// source points coincide (then no scale can be found), or a coordinate is
/// not finite.
///
/// The derivation: the best translation makes the centroid (mean point) of
/// the transformed source points land on the centroid of the destination
/// points. So centre both sets on their centroids, giving `p` and `q`, and
/// only the linear part `a + i*b` remains. Minimising the sum of squared
/// distances `|(a + i*b) * p - q|^2` is a linear least-squares problem in
/// `a` and `b`, and setting the derivatives to zero gives
///
/// ```text
/// a = sum(px*qx + py*qy) / sum(px*px + py*py)
/// b = sum(px*qy - py*qx) / sum(px*px + py*py)
/// ```
///
/// The numerators are the sums of the dot product and the cross product of
/// each pair of points, so no square root is needed. Finally `tx, ty` are
/// chosen so that the source centroid maps to the destination centroid.
pub fn fit(src: &[[f32; 2]], dst: &[[f32; 2]]) -> Option<Similarity> {
    if src.len() != dst.len() || src.len() < 2 {
        return None;
    }
    let count = src.len() as f32;
    let mut src_centroid = [0.0f32; 2];
    let mut dst_centroid = [0.0f32; 2];
    for (s, d) in src.iter().zip(dst) {
        src_centroid[0] += s[0];
        src_centroid[1] += s[1];
        dst_centroid[0] += d[0];
        dst_centroid[1] += d[1];
    }
    src_centroid[0] /= count;
    src_centroid[1] /= count;
    dst_centroid[0] /= count;
    dst_centroid[1] /= count;

    // `spread` is the sum of the squared lengths of the centred source
    // points, `dot` and `cross` the sums of the products with the centred
    // destination points.
    let mut spread = 0.0f32;
    let mut dot = 0.0f32;
    let mut cross = 0.0f32;
    for (s, d) in src.iter().zip(dst) {
        let px = s[0] - src_centroid[0];
        let py = s[1] - src_centroid[1];
        let qx = d[0] - dst_centroid[0];
        let qy = d[1] - dst_centroid[1];
        spread += px * px + py * py;
        dot += px * qx + py * qy;
        cross += px * qy - py * qx;
    }
    if !spread.is_finite() || spread <= 0.0 || !dot.is_finite() || !cross.is_finite() {
        return None;
    }
    let a = dot / spread;
    let b = cross / spread;
    Some(Similarity {
        a,
        b,
        tx: dst_centroid[0] - (a * src_centroid[0] - b * src_centroid[1]),
        ty: dst_centroid[1] - (b * src_centroid[0] + a * src_centroid[1]),
    })
}

/// The standard positions of the five landmarks in an aligned 112x112 face
/// crop, as used by ArcFace and most other face recognizers.
///
/// The order is: the eye on the image's left, the eye on the image's right,
/// the nose tip, the mouth corner on the image's left, the mouth corner on
/// the image's right. "Image's left" means the smaller x, which is the
/// person's right eye when they face the camera. A detector's landmarks must
/// be passed to [`fit`] in the same order.
pub const ARCFACE_TEMPLATE_112: [[f32; 2]; 5] = [
    [38.2946, 51.6963],
    [73.5318, 51.5014],
    [56.0252, 71.7366],
    [41.5493, 92.3655],
    [70.7299, 92.2041],
];

/// The largest integer not above `value`, like `f32::floor` which is not
/// available without `std`.
///
/// `as i32` truncates toward zero, so `-0.5` becomes `0`; floor needs `-1`.
/// When the truncated value is above the input, the input was negative with
/// a fraction, and one is subtracted. Values outside the `i32` range
/// saturate, which is fine for pixel coordinates.
fn floor_to_i32(value: f32) -> i32 {
    let truncated = value as i32;
    if truncated as f32 > value {
        truncated - 1
    } else {
        truncated
    }
}

/// Sample `src` at the fractional position `(sx, sy)` with bilinear
/// interpolation, or `None` when a needed source pixel is outside the image
/// or the position is not finite.
///
/// The value is a weighted mean of the four pixels around the position,
/// where the weights are the areas of the opposite rectangles:
///
/// ```text
/// (1-fx)(1-fy) * p00 + fx(1-fy) * p10 + (1-fx)fy * p01 + fx fy * p11
/// ```
///
/// with `fx, fy` the fractions of the position past its floor. When a
/// fraction is exactly 0, the pixels on the far side have weight 0 and are
/// not needed, so positions on the last row or column are still inside.
/// This keeps the identity transform an exact copy.
fn sample_bilinear<const CHANNELS: usize>(
    src: &Image<'_, CHANNELS>,
    sx: f32,
    sy: f32,
) -> Option<[u8; CHANNELS]> {
    // The image is empty, or the position is outside. `contains` is false
    // for NaN too. After this check the floors are between 0 and the last
    // index, so the casts below cannot fail or overflow.
    if src.width() == 0 || src.height() == 0 {
        return None;
    }
    let max_x = (src.width() - 1) as f32;
    let max_y = (src.height() - 1) as f32;
    if !(0.0..=max_x).contains(&sx) || !(0.0..=max_y).contains(&sy) {
        return None;
    }
    let x0 = floor_to_i32(sx);
    let y0 = floor_to_i32(sy);
    let fx = sx - x0 as f32;
    let fy = sy - y0 as f32;
    let x0 = usize::try_from(x0).ok()?;
    let y0 = usize::try_from(y0).ok()?;
    // The far column and row, or the near one when its weight is zero.
    let x1 = if fx > 0.0 { x0 + 1 } else { x0 };
    let y1 = if fy > 0.0 { y0 + 1 } else { y0 };
    if x1 >= src.width() || y1 >= src.height() {
        return None;
    }
    let p00 = src.pixel(x0, y0);
    let p10 = src.pixel(x1, y0);
    let p01 = src.pixel(x0, y1);
    let p11 = src.pixel(x1, y1);
    let w00 = (1.0 - fx) * (1.0 - fy);
    let w10 = fx * (1.0 - fy);
    let w01 = (1.0 - fx) * fy;
    let w11 = fx * fy;
    let mut values = [0u8; CHANNELS];
    for (channel, value) in values.iter_mut().enumerate() {
        let mixed = w00 * f32::from(p00[channel])
            + w10 * f32::from(p10[channel])
            + w01 * f32::from(p01[channel])
            + w11 * f32::from(p11[channel]);
        // `mixed` is a weighted mean of bytes, so it is between 0 and 255.
        // Adding 0.5 and truncating rounds to the nearest value.
        *value = (mixed + 0.5) as u8;
    }
    Some(values)
}

/// Draw `dst` by sampling `src` through `dst_to_src`.
///
/// For every destination pixel `(x, y)`, taken at its integer coordinates
/// with no half-pixel offset (like OpenCV's `warpAffine`), the transform
/// gives a position in the source. The pixel gets the bilinear sample at
/// that position, rounded to the nearest value per channel. Where any of the
/// needed source pixels lies outside the source, the pixel is 0 in every
/// channel: a black border.
///
/// `dst` can have any size; it is the crop's size. Both images have the same
/// number of channels, so a gray source gives a gray crop and an RGB source
/// an RGB crop.
pub fn warp<const CHANNELS: usize>(
    src: &Image<'_, CHANNELS>,
    dst_to_src: &Similarity,
    dst: &mut ImageMut<'_, CHANNELS>,
) {
    for y in 0..dst.height() {
        for x in 0..dst.width() {
            let [sx, sy] = dst_to_src.apply([x as f32, y as f32]);
            let values = sample_bilinear(src, sx, sy).unwrap_or([0; CHANNELS]);
            dst.set_pixel(x, y, values);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floor_rounds_toward_negative_infinity() {
        assert_eq!(floor_to_i32(2.7), 2);
        assert_eq!(floor_to_i32(2.0), 2);
        assert_eq!(floor_to_i32(0.0), 0);
        assert_eq!(floor_to_i32(-0.5), -1);
        assert_eq!(floor_to_i32(-1.0), -1);
        assert_eq!(floor_to_i32(-1.2), -2);
    }

    #[test]
    fn sampling_between_two_pixels_mixes_them() {
        let data = [0u8, 100, 200, 44];
        let image = Image::<1>::new(&data, 2, 2);
        assert_eq!(sample_bilinear(&image, 0.5, 0.0), Some([50]));
        assert_eq!(sample_bilinear(&image, 0.0, 0.5), Some([100]));
        assert_eq!(sample_bilinear(&image, 1.0, 1.0), Some([44]));
        assert_eq!(sample_bilinear(&image, 1.01, 1.0), None);
        assert_eq!(sample_bilinear(&image, -0.01, 0.0), None);
        assert_eq!(sample_bilinear(&image, f32::NAN, 0.0), None);
    }
}
