//! From a detected face to the recognizer's input: cut the face out of the
//! frame so that its landmarks land where the recognizer expects them.
//!
//! The recognizer was trained on 112x112 crops in which the eyes, nose
//! and mouth corners sit at fixed positions (`warp::ARCFACE_TEMPLATE_112`).
//! [`align_face`] finds the rotation, scale and shift that moves the
//! detected landmarks onto those positions, and resamples the frame
//! through it. Every face then arrives at the recognizer in the same
//! pose, which is the biggest single help to its accuracy.

use core::ops::Range;

use crate::{
    image::{RgbImage, RgbImageMut},
    warp::{self, ARCFACE_TEMPLATE_112, Similarity},
};

/// Rows and columns of the aligned crop: the size of the ArcFace
/// template, and of the recognizer's input (`nn::mfn`).
pub const CROP_SIZE: usize = 112;

/// Cut the face with `landmarks` (in pixels of `source`) out of `source`
/// into `crop`, a `CROP_SIZE` x `CROP_SIZE` RGB image. Returns the
/// transform from `source` pixels to `crop` pixels, or `None` when the
/// landmarks do not define one (all on one point).
///
/// # Panics
///
/// When `crop` is not `CROP_SIZE` x `CROP_SIZE`.
pub fn align_face(
    landmarks: &[[f32; 2]; 5],
    source: &RgbImage<'_>,
    crop: &mut RgbImageMut<'_>,
) -> Option<Similarity> {
    assert_eq!(
        (crop.width(), crop.height()),
        (CROP_SIZE, CROP_SIZE),
        "crop size"
    );
    let source_to_crop = warp::fit(landmarks, &ARCFACE_TEMPLATE_112)?;
    let crop_to_source = source_to_crop.inverse()?;
    warp::warp(source, &crop_to_source, crop);
    Some(source_to_crop)
}

/// The pixels of a source of `width` x `height` that [`align_face`] reads
/// for these landmarks: a range of columns and a range of rows (see
/// `warp::footprint`). `None` when the landmarks do not define a
/// transform, as for `align_face`.
///
/// The application scales the camera frame down only there: the rest of
/// the scaled-down image is never read.
pub fn source_region(
    landmarks: &[[f32; 2]; 5],
    width: usize,
    height: usize,
) -> Option<(Range<usize>, Range<usize>)> {
    let crop_to_source = warp::fit(landmarks, &ARCFACE_TEMPLATE_112)?.inverse()?;
    warp::footprint(&crop_to_source, CROP_SIZE, CROP_SIZE, width, height)
}
