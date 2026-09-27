//! From a detected face to the recognizer's input: cut the face out of the
//! frame so that its landmarks land where the recognizer expects them.
//!
//! The recognizer was trained on 112x112 crops in which the eyes, nose
//! and mouth corners sit at fixed positions (`warp::ARCFACE_TEMPLATE_112`).
//! [`align_face`] finds the rotation, scale and shift that moves the
//! detected landmarks onto those positions, and resamples the frame
//! through it. Every face then arrives at the recognizer in the same
//! pose, which is the biggest single help to its accuracy.

use crate::{
    image::{RgbImage, RgbImageMut},
    nn::edgeface::{INPUT_SIZE, int8::INPUT_QUANT},
    warp::{self, ARCFACE_TEMPLATE_112, Similarity},
};

/// Rows and columns of the aligned crop.
pub const CROP_SIZE: usize = INPUT_SIZE;

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

/// The recognizer's input from an aligned crop: `(byte / 255 - 0.5) / 0.5`
/// per channel, R, G, B, channels-last, `CROP_SIZE * CROP_SIZE * 3`
/// values.
///
/// # Panics
///
/// When the crop or the buffer has the wrong size.
pub fn recognizer_input(crop: &RgbImage<'_>, input: &mut [f32]) {
    assert_eq!(
        (crop.width(), crop.height()),
        (CROP_SIZE, CROP_SIZE),
        "crop size"
    );
    let input = &mut input[..CROP_SIZE * CROP_SIZE * 3];
    for (value, &byte) in input.iter_mut().zip(crop.data()) {
        *value = (f32::from(byte) / 255.0 - 0.5) / 0.5;
    }
}

/// The integer recognizer's input from an aligned crop: the same values
/// as [`recognizer_input`], quantized with `edgeface::int8::INPUT_QUANT`.
///
/// # Panics
///
/// When the crop or the buffer has the wrong size.
pub fn recognizer_input_i8(crop: &RgbImage<'_>, input: &mut [i8]) {
    assert_eq!(
        (crop.width(), crop.height()),
        (CROP_SIZE, CROP_SIZE),
        "crop size"
    );
    let input = &mut input[..CROP_SIZE * CROP_SIZE * 3];
    for (value, &byte) in input.iter_mut().zip(crop.data()) {
        *value = INPUT_QUANT.quantize((f32::from(byte) / 255.0 - 0.5) / 0.5);
    }
}
