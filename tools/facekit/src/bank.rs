//! The `bank` command: the impostor bank the firmware carries.
//!
//! A bare threshold on similarity is fragile, because every score drifts
//! together when the light or the camera gain changes. The firmware
//! therefore also asks: is this face more like the enrolled person than
//! like the most similar stranger we know of? The strangers are a few
//! hundred embeddings computed here and baked into the weights file.
//!
//! The faces come from a folder with one subfolder of photos per person
//! (the layout of Labeled Faces in the Wild). One photo per person is
//! used, so the bank holds that many different people. They are chosen
//! evenly across the folder listing, not the first N, so that an
//! alphabetical run of one nationality does not dominate.
//!
//! The embeddings are stored as `i8` with one scale for the whole bank:
//! 512 bytes per person instead of 2 KB, and the rounding changes a
//! cosine by less than 0.002.

use std::{fs, path::Path};

use anyhow::{Context, Result, bail};
use hack_and_hike_vision::{
    blob::DataType,
    nn::{mfn::EMBEDDING_LEN, quant::Quant},
};
use sha2::{Digest, Sha256};

use crate::{
    blob::{Tensor, Writer},
    data,
    embed::{Pick, embed_all},
    mfn::Recognizer,
};

/// Write an impostor bank of `count` people to `out`.
///
/// # Errors
///
/// When the folders cannot be read, too few faces are found, or the file
/// cannot be written.
pub fn run(
    detector: &Path,
    weights: &Path,
    images: &Path,
    out: &Path,
    count: usize,
) -> Result<()> {
    // One photo per person, spread evenly over the listing.
    let people = data::people(images)?;
    if people.len() < count {
        bail!(
            "{} people in {}, need {count}",
            people.len(),
            images.display()
        );
    }
    let step = people.len() / count;
    let mut photos = Vec::with_capacity(count);
    for index in 0..count {
        let person = &people[index * step];
        let mut shots: Vec<_> = fs::read_dir(person)?
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| {
                path.extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("jpg"))
            })
            .collect();
        shots.sort();
        if let Some(first) = shots.into_iter().next() {
            photos.push(first);
        }
    }
    println!("{} people, one photo each", photos.len());

    let recognizer = Recognizer::load(weights, None)?;
    let embeddings = embed_all(detector, &recognizer, &photos, Pick::Centre)?;
    println!("{} of them have a face", embeddings.len());
    if embeddings.len() < count / 2 {
        bail!(
            "only {} faces found, expected about {count}",
            embeddings.len()
        );
    }

    let mut vectors: Vec<Vec<f32>> = photos
        .iter()
        .filter_map(|path| embeddings.get(path).map(|e| e.integer.clone()))
        .collect();
    vectors.sort_by(|a, b| a[0].total_cmp(&b[0]));

    let bound = vectors
        .iter()
        .flat_map(|v| v.iter())
        .fold(0.0f32, |max, value| max.max(value.abs()));
    let quant = Quant {
        scale: bound / 127.0,
        zero_point: 0,
    };
    let data: Vec<u8> = vectors
        .iter()
        .flat_map(|v| v.iter().map(|&value| quant.quantize(value) as u8))
        .collect();

    // The largest cosine the rounding can move, as a check on the scale.
    let worst = vectors
        .iter()
        .map(|v| {
            let back: Vec<f32> = v
                .iter()
                .map(|&x| quant.dequantize(quant.quantize(x)))
                .collect();
            let dot: f32 = v.iter().zip(&back).map(|(a, b)| a * b).sum();
            let norm = back.iter().map(|x| x * x).sum::<f32>().sqrt();
            1.0 - dot / norm.max(1e-12)
        })
        .fold(0.0f32, f32::max);
    println!("i8 rounding moves a cosine by at most {worst:.4}");

    let mut writer = Writer::new();
    writer.add(Tensor {
        name: "impostors".to_string(),
        layout: "NC".to_string(),
        data_type: DataType::I8,
        dims: vec![vectors.len(), EMBEDDING_LEN],
        scale: quant.scale,
        zero_point: 0,
        data,
    })?;
    let bytes = writer.finish();
    fs::write(out, &bytes).with_context(|| format!("writing {}", out.display()))?;
    println!(
        "wrote {} ({} embeddings, {} bytes, sha256 {:x})",
        out.display(),
        vectors.len(),
        bytes.len(),
        Sha256::digest(&bytes)
    );
    Ok(())
}
