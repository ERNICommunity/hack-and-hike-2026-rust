//! The `crops` command: calibration and evaluation data from photos.
//!
//! For every photo in a folder, find the faces with YuNet (run by tract at
//! the photo's size, which the board cannot do) and write, for each face:
//!
//! - `crops/<photo>_<n>.png`: the 112x112 aligned crop, made with the same
//!   `align_face` the board uses, from the full-resolution photo;
//! - `frames/<photo>_<n>.png`: a 320x240 image with the face filling 90
//!   percent of the height, like a camera frame of a user who follows the
//!   application's framing rule.
//!
//! The decoding and alignment are the firmware's own code
//! (`hack_and_hike_vision::detect`, `::align`); only the detector itself
//! runs in tract here.

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use hack_and_hike_vision::{
    align::{CROP_SIZE, align_face},
    detect::{DEFAULT_NMS_THRESHOLD, Face, decode},
    image::{RgbImage, RgbImageMut},
    nn::{Shape, yunet::Heads},
};
use image::{RgbImage as Photo, imageops::FilterType};

use crate::{data, onnx::Runner};

/// The longest side of the image the detector sees.
const DETECT_MAX_SIDE: u32 = 640;
/// Faces below this score are ignored.
const MIN_SCORE: f32 = 0.8;
/// The face's share of the frame height.
const FRAME_FACE_FRACTION: f32 = 0.9;

/// The names of the detector's outputs, in the order the runner returns
/// them.
const OUTPUTS: [&str; 12] = [
    "cls_8", "cls_16", "cls_32", "obj_8", "obj_16", "obj_32", "bbox_8", "bbox_16", "bbox_32",
    "kps_8", "kps_16", "kps_32",
];

/// YuNet in tract at a photo's own size: what the board cannot do, and
/// what `crops` and `eval` need in order to find faces in photos that are
/// much larger than the board's 96x64 detector input.
///
/// The decoding is the firmware's own ([`decode`]), so only the network
/// runs here; the boxes and landmarks come back in the photo's own
/// pixels. One runner is built per input size and kept, because building
/// one takes far longer than running it.
pub struct Detector {
    /// The ONNX file.
    model: PathBuf,
    /// A runner per padded input size.
    runners: HashMap<(usize, usize), Runner>,
}

impl Detector {
    /// A detector that loads `model` as sizes come up.
    pub fn new(model: &Path) -> Self {
        Self {
            model: model.to_path_buf(),
            runners: HashMap::new(),
        }
    }

    /// The faces of `photo`, in its own pixels, best first.
    ///
    /// # Errors
    ///
    /// When tract cannot load or run the model.
    pub fn detect(&mut self, photo: &Photo) -> Result<Vec<Face>> {
        // The detector sees the photo scaled to at most 640 on the long
        // side, padded with black to multiples of 32 (its neck halves the
        // map five times).
        let scale = (DETECT_MAX_SIDE as f32 / photo.width().max(photo.height()) as f32).min(1.0);
        let (width, height) = (
            ((photo.width() as f32 * scale).round() as usize).max(32),
            ((photo.height() as f32 * scale).round() as usize).max(32),
        );
        let (padded_width, padded_height) =
            (width.next_multiple_of(32), height.next_multiple_of(32));
        let small =
            image::imageops::resize(photo, width as u32, height as u32, FilterType::Triangle);
        let shape = [1, 3, padded_height, padded_width];
        let mut input = vec![0.0f32; 3 * padded_height * padded_width];
        for y in 0..height {
            for x in 0..width {
                let pixel = small.get_pixel(x as u32, y as u32).0;
                for channel in 0..3 {
                    // BGR planes, 0..255.
                    input[((2 - channel) * padded_height + y) * padded_width + x] =
                        f32::from(pixel[channel]);
                }
            }
        }
        let outputs: Vec<String> = OUTPUTS.iter().map(|name| name.to_string()).collect();
        let runner = match self.runners.entry((padded_height, padded_width)) {
            std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(Runner::new(&self.model, &shape, &outputs)?)
            }
        };
        let results = runner.run(&shape, &input)?;
        let by_name: HashMap<&str, &[f32]> = results
            .iter()
            .map(|output| (output.name.as_str(), output.values.as_slice()))
            .collect();
        let heads: [Heads<'_>; 3] = [8usize, 16, 32].map(|stride| Heads {
            stride,
            map: Shape::new(padded_height / stride, padded_width / stride, 1),
            cls: by_name[format!("cls_{stride}").as_str()],
            obj: by_name[format!("obj_{stride}").as_str()],
            bbox: by_name[format!("bbox_{stride}").as_str()],
            kps: by_name[format!("kps_{stride}").as_str()],
        });
        // Back to the photo's own pixels.
        let faces = decode(&heads, MIN_SCORE, DEFAULT_NMS_THRESHOLD);
        Ok(faces.iter().map(|face| face.scaled(1.0 / scale)).collect())
    }
}

/// Write crops and frames for every face in every photo of `photos`.
///
/// # Errors
///
/// When a folder cannot be read or written, or tract fails.
pub fn run(model: &Path, photos: &Path, out: &Path) -> Result<()> {
    let entries = data::photos(photos)?;
    let crops_dir = out.join("crops");
    let frames_dir = out.join("frames");
    fs::create_dir_all(&crops_dir)?;
    fs::create_dir_all(&frames_dir)?;
    let mut detector = Detector::new(model);

    let mut total = 0;
    for path in &entries {
        let photo = match image::open(path) {
            Ok(photo) => photo.to_rgb8(),
            Err(error) => {
                println!("skip {}: {error}", path.display());
                continue;
            }
        };
        let stem = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("photo");
        let faces = detector.detect(&photo)?;
        let full = RgbImage::new(
            photo.as_raw(),
            photo.width() as usize,
            photo.height() as usize,
        );
        for (index, face) in faces.iter().enumerate() {
            let mut crop = vec![0u8; CROP_SIZE * CROP_SIZE * 3];
            let mut crop_image = RgbImageMut::new(&mut crop, CROP_SIZE, CROP_SIZE);
            if align_face(&face.landmarks, &full, &mut crop_image).is_none() {
                continue;
            }
            let name = format!("{stem}_{index}.png");
            Photo::from_raw(CROP_SIZE as u32, CROP_SIZE as u32, crop)
                .context("crop image")?
                .save(crops_dir.join(&name))?;
            if let Some(frame) = frame_around(&photo, face) {
                frame.save(frames_dir.join(&name))?;
            }
            total += 1;
            println!(
                "{}: face {index} score {:.2} -> {name}",
                path.display(),
                face.score
            );
        }
    }
    println!(
        "{total} faces from {} photos into {}",
        entries.len(),
        out.display()
    );
    Ok(())
}

/// A 320x240 image with `face` filling 90 percent of the height, or
/// `None` when the photo does not have enough room around the face.
fn frame_around(photo: &Photo, face: &Face) -> Option<Photo> {
    let height = face.height / FRAME_FACE_FRACTION;
    let width = height * 4.0 / 3.0;
    let [cx, cy] = face.centre();
    let (left, top) = (cx - width / 2.0, cy - height / 2.0);
    if left < 0.0
        || top < 0.0
        || left + width > photo.width() as f32
        || top + height > photo.height() as f32
    {
        return None;
    }
    let window =
        image::imageops::crop_imm(photo, left as u32, top as u32, width as u32, height as u32)
            .to_image();
    Some(image::imageops::resize(
        &window,
        320,
        240,
        FilterType::Triangle,
    ))
}
