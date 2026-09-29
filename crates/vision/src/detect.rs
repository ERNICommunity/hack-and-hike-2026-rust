//! From the detector's raw outputs to faces: decoding, non-maximum
//! suppression, and the detector's input from a camera frame.
//!
//! YuNet predicts, for every cell of three grids (strides 8, 16 and 32
//! pixels), whether a face is centred there, how to move and scale a box
//! of the stride's size to fit it, and where its five landmarks are. This
//! module turns those predictions into [`Face`]s the way OpenCV's
//! `FaceDetectorYN` does, so the numbers mean the same as in every YuNet
//! example:
//!
//! ```text
//! score  = sqrt(cls * obj)
//! centre = (column + dx, row + dy) * stride
//! size   = (exp(dw), exp(dh)) * stride
//! point  = (column + kx, row + ky) * stride
//! ```
//!
//! Coordinates are pixels of the detector input, a 96x64 image that holds
//! the camera frame scaled down by [`DOWNSCALE`] in its top-left corner
//! (see [`detector_input`]). Multiply by [`DOWNSCALE`] to get frame
//! coordinates ([`Face::scaled`]).

use libm::{expf, sqrtf};

use crate::{
    image::RgbImage,
    nn::yunet::{self, Heads, INPUT_HEIGHT, INPUT_WIDTH},
};

/// The camera frame is scaled down by this factor for the detector:
/// 320x240 becomes 80x60.
pub const DOWNSCALE: usize = 4;
/// Columns of the detector input that hold the frame.
pub const CONTENT_WIDTH: usize = 320 / DOWNSCALE;
/// Rows of the detector input that hold the frame.
pub const CONTENT_HEIGHT: usize = 240 / DOWNSCALE;
/// The most faces [`decode`] returns.
pub const MAX_FACES: usize = 8;
/// The most cells kept before non-maximum suppression. A frame with one
/// face fills a handful of neighbouring cells.
const MAX_CANDIDATES: usize = 32;
/// OpenCV's default score threshold for YuNet.
pub const DEFAULT_SCORE_THRESHOLD: f32 = 0.9;
/// OpenCV's default overlap threshold for YuNet's non-maximum suppression.
pub const DEFAULT_NMS_THRESHOLD: f32 = 0.3;

/// One detected face, in pixels of the image the detector saw.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Face {
    /// Confidence, 0 to 1.
    pub score: f32,
    /// Left edge of the box.
    pub x: f32,
    /// Top edge of the box.
    pub y: f32,
    /// Width of the box.
    pub width: f32,
    /// Height of the box.
    pub height: f32,
    /// The five landmarks, as `[x, y]`: the eye on the image's left, the
    /// eye on the image's right, the nose tip, the mouth corner on the
    /// image's left, the mouth corner on the image's right. This is the
    /// order of the alignment template, `warp::ARCFACE_TEMPLATE_112`.
    pub landmarks: [[f32; 2]; 5],
}

impl Face {
    /// The same face with every coordinate multiplied by `factor`, for
    /// example [`DOWNSCALE`] to go from detector to frame pixels.
    pub fn scaled(&self, factor: f32) -> Self {
        let mut landmarks = self.landmarks;
        for point in &mut landmarks {
            point[0] *= factor;
            point[1] *= factor;
        }
        Self {
            score: self.score,
            x: self.x * factor,
            y: self.y * factor,
            width: self.width * factor,
            height: self.height * factor,
            landmarks,
        }
    }

    /// The centre of the box.
    pub fn centre(&self) -> [f32; 2] {
        [self.x + self.width / 2.0, self.y + self.height / 2.0]
    }

    /// Intersection over union with `other`: 0 when the boxes do not
    /// overlap, 1 when they are the same.
    pub fn overlap(&self, other: &Face) -> f32 {
        let left = self.x.max(other.x);
        let top = self.y.max(other.y);
        let right = (self.x + self.width).min(other.x + other.width);
        let bottom = (self.y + self.height).min(other.y + other.height);
        let intersection = (right - left).max(0.0) * (bottom - top).max(0.0);
        let union = self.width * self.height + other.width * other.height - intersection;
        if union <= 0.0 {
            0.0
        } else {
            intersection / union
        }
    }
}

/// The faces of one frame, best first. Dereferences to a slice.
#[derive(Clone, Copy, Debug)]
pub struct Faces {
    /// The faces; only the first `count` are meaningful.
    faces: [Face; MAX_FACES],
    /// How many faces there are.
    count: usize,
}

impl core::ops::Deref for Faces {
    type Target = [Face];

    fn deref(&self) -> &[Face] {
        &self.faces[..self.count]
    }
}

impl Faces {
    /// The best face, if any.
    pub fn best(&self) -> Option<&Face> {
        self.first()
    }
}

/// A fixed-size list of faces sorted by score, best first. A new face
/// that scores below every entry of a full list is dropped.
struct Candidates {
    /// The faces, sorted.
    faces: [Face; MAX_CANDIDATES],
    /// How many are used.
    count: usize,
}

impl Candidates {
    /// Insert `face` at its place in the order.
    fn insert(&mut self, face: Face) {
        let mut position = self.count;
        while position > 0 && self.faces[position - 1].score < face.score {
            position -= 1;
        }
        if position == MAX_CANDIDATES {
            return;
        }
        let end = (self.count + 1).min(MAX_CANDIDATES);
        self.faces.copy_within(position..end - 1, position + 1);
        self.faces[position] = face;
        self.count = end;
    }
}

/// Turn the detector's raw outputs into faces: every grid cell with a
/// score of at least `score_threshold` becomes a candidate, and
/// non-maximum suppression drops a candidate that overlaps a better one
/// by more than `nms_threshold` (intersection over union).
pub fn decode(heads: &[Heads<'_>; 3], score_threshold: f32, nms_threshold: f32) -> Faces {
    let mut candidates = Candidates {
        faces: [Face::default(); MAX_CANDIDATES],
        count: 0,
    };
    for head in heads {
        let stride = head.stride as f32;
        let columns = head.map.width;
        for (cell, (cls, obj)) in head.cls.iter().zip(head.obj).enumerate() {
            let score = sqrtf(cls.clamp(0.0, 1.0) * obj.clamp(0.0, 1.0));
            if score < score_threshold {
                continue;
            }
            let (column, row) = ((cell % columns) as f32, (cell / columns) as f32);
            let bbox = &head.bbox[cell * yunet::BBOX_LEN..(cell + 1) * yunet::BBOX_LEN];
            let kps = &head.kps[cell * yunet::KPS_LEN..(cell + 1) * yunet::KPS_LEN];
            let (cx, cy) = ((column + bbox[0]) * stride, (row + bbox[1]) * stride);
            let (width, height) = (expf(bbox[2]) * stride, expf(bbox[3]) * stride);
            let mut landmarks = [[0.0f32; 2]; 5];
            for (point, pair) in landmarks.iter_mut().zip(kps.chunks_exact(2)) {
                *point = [(pair[0] + column) * stride, (pair[1] + row) * stride];
            }
            candidates.insert(Face {
                score,
                x: cx - width / 2.0,
                y: cy - height / 2.0,
                width,
                height,
                landmarks,
            });
        }
    }

    // Greedy non-maximum suppression: keep the best, drop what overlaps
    // it, repeat with the next survivor.
    let mut faces = Faces {
        faces: [Face::default(); MAX_FACES],
        count: 0,
    };
    for index in 0..candidates.count {
        let face = candidates.faces[index];
        let suppressed = faces.iter().any(|kept| kept.overlap(&face) > nms_threshold);
        if !suppressed && faces.count < MAX_FACES {
            faces.faces[faces.count] = face;
            faces.count += 1;
        }
    }
    faces
}

/// Fill `input` (the detector's `f32` input, `INPUT_HEIGHT` x
/// `INPUT_WIDTH` x 3) from the scaled-down frame: `frame` goes to the
/// top-left corner in B, G, R order with values 0 to 255, the rest is
/// black.
///
/// # Panics
///
/// When `frame` is larger than the input, or `input` is too short.
pub fn detector_input(frame: &RgbImage<'_>, input: &mut [f32]) {
    let shape = yunet::INPUT_SHAPE;
    assert!(
        frame.width() <= INPUT_WIDTH && frame.height() <= INPUT_HEIGHT,
        "frame larger than the detector input"
    );
    let input = &mut input[..shape.len()];
    input.fill(0.0);
    for y in 0..frame.height() {
        for x in 0..frame.width() {
            let [r, g, b] = frame.pixel(x, y);
            let slot = &mut input[shape.offset(x, y)..shape.offset(x, y) + 3];
            slot.copy_from_slice(&[f32::from(b), f32::from(g), f32::from(r)]);
        }
    }
}

/// Fill `input` (the detector's `i8` input, `INPUT_HEIGHT` x
/// `INPUT_WIDTH` x 3) from the scaled-down frame, like [`detector_input`]
/// but for the integer detector (`yunet::int8`): every byte becomes
/// `byte - 128`, so 0..255 maps to -128..127, and the black padding is
/// `-128`. This is exactly [`detector_input`]'s output quantized with
/// `yunet::int8::INPUT_QUANT` (scale 1, zero point -128).
///
/// # Panics
///
/// When `frame` is larger than the input, or `input` is too short.
pub fn detector_input_i8(frame: &RgbImage<'_>, input: &mut [i8]) {
    let shape = yunet::INPUT_SHAPE;
    assert!(
        frame.width() <= INPUT_WIDTH && frame.height() <= INPUT_HEIGHT,
        "frame larger than the detector input"
    );
    let input = &mut input[..shape.len()];
    input.fill(-128);
    let row_len = shape.width * shape.channels;
    for (y, row) in input
        .chunks_exact_mut(row_len)
        .take(frame.height())
        .enumerate()
    {
        for (slot, rgb) in row.chunks_exact_mut(3).zip(frame.row(y).chunks_exact(3)) {
            // `byte - 128` is the byte with its top bit flipped.
            slot[0] = (rgb[2] ^ 0x80) as i8;
            slot[1] = (rgb[1] ^ 0x80) as i8;
            slot[2] = (rgb[0] ^ 0x80) as i8;
        }
    }
}
