//! Embedding photos with the firmware's own pipeline, on many threads.
//!
//! `eval`, `bank` and `calibrate` all need the same thing: take a photo,
//! find the face, align it the way the board does, and run the
//! recognizer. This module does that once, for all of them.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use anyhow::Result;
use hack_and_hike_vision::{
    align::{CROP_SIZE, align_face},
    image::RgbImage,
};
use rayon::prelude::*;

use crate::{
    crops::Detector,
    mfn::{Buffers, Recognizer},
};

/// Which face of a photo to measure, when the photo holds more than one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum Pick {
    /// The one the detector is most sure of.
    Score,
    /// The one whose box centre is nearest the photo's centre. In LFW the
    /// subject is centred by construction and bystanders are not.
    Centre,
    /// The one with the largest box.
    Largest,
}

/// The embeddings of one photo: the board's, and the `.espdl`
/// interpreter's when the recognizer has a reference.
#[derive(Clone)]
pub struct Embeddings {
    /// The embedding as the board computes it, L2-normalized.
    pub integer: Vec<f32>,
    /// The interpreter's, L2-normalized; empty without a reference.
    pub reference: Vec<f32>,
}

/// Embed every photo, in parallel. Photos without a face are left out.
pub fn embed_all(
    detector: &Path,
    recognizer: &Recognizer,
    photos: &[PathBuf],
    pick: Pick,
) -> Result<HashMap<PathBuf, Embeddings>> {
    let done = std::sync::atomic::AtomicUsize::new(0);
    let results: Vec<Option<(PathBuf, Embeddings)>> = photos
        .par_iter()
        .map_init(
            || Worker::new(detector),
            |worker, path| {
                let worker = worker.as_mut().ok()?;
                let count = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if count.is_multiple_of(2000) {
                    println!("  {count} of {}", photos.len());
                }
                worker
                    .embed(recognizer, path, pick)
                    .map(|e| (path.clone(), e))
            },
        )
        .collect();
    Ok(results.into_iter().flatten().collect())
}

/// One thread's detector and buffers.
pub struct Worker {
    /// The detector, at the size of an LFW photo.
    detector: Detector,
    /// The recognizer's working memory.
    buffers: Buffers,
}

impl Worker {
    /// Load the detector and allocate the buffers.
    pub fn new(detector: &Path) -> Result<Self> {
        Ok(Self {
            detector: Detector::new(detector),
            buffers: Buffers::default(),
        })
    }

    /// The embeddings of the best face in `path`, or `None` when the
    /// photo cannot be read or holds no face.
    fn embed(&mut self, recognizer: &Recognizer, path: &Path, pick: Pick) -> Option<Embeddings> {
        let photo = image::open(path).ok()?.to_rgb8();
        let (width, height) = (photo.width() as usize, photo.height() as usize);
        let faces = self.detector.detect(&photo).ok()?;
        let centre = [width as f32 / 2.0, height as f32 / 2.0];
        let face = match pick {
            Pick::Score => faces.first()?,
            Pick::Centre => faces.iter().min_by(|a, b| {
                let distance = |f: &hack_and_hike_vision::detect::Face| {
                    let c = f.centre();
                    (c[0] - centre[0]).powi(2) + (c[1] - centre[1]).powi(2)
                };
                distance(a).total_cmp(&distance(b))
            })?,
            Pick::Largest => faces
                .iter()
                .max_by(|a, b| (a.width * a.height).total_cmp(&(b.width * b.height)))?,
        };
        let source = RgbImage::new(photo.as_raw(), width, height);
        let mut crop = vec![0u8; CROP_SIZE * CROP_SIZE * 3];
        let mut crop_image =
            hack_and_hike_vision::image::RgbImageMut::new(&mut crop, CROP_SIZE, CROP_SIZE);
        align_face(&face.landmarks, &source, &mut crop_image)?;
        let crop = crop_image.as_image();

        let integer = recognizer.embed(&crop, &mut self.buffers);
        let reference = recognizer.reference(&crop).unwrap_or_default();
        Some(Embeddings { integer, reference })
    }
}

/// Divide a vector by its length.
pub fn normalize(values: &mut [f32]) {
    let norm = values.iter().map(|v| v * v).sum::<f32>().sqrt().max(1e-12);
    for value in values.iter_mut() {
        *value /= norm;
    }
}

/// The dot product of two unit vectors.
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}
