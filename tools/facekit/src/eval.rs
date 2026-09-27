//! The `eval` command: how well does the pipeline tell faces apart?
//!
//! The golden vectors of Step 3 prove that the firmware computes the same
//! numbers as the original model. They cannot say whether those numbers
//! are any good at recognizing faces: a wrong landmark order or a badly
//! chosen crop would pass them and still ruin the application. This
//! command answers the other question, on Labeled Faces in the Wild
//! (LFW), the benchmark that the model's authors published their accuracy
//! on.
//!
//! LFW ships 6,000 pairs of photos in ten folds: in each fold, 300 pairs
//! show the same person and 300 show two different people. The published
//! protocol is ten-fold cross-validation: pick the similarity threshold
//! on nine folds, measure the accuracy on the tenth, repeat ten times,
//! report the mean and the spread. `eval` runs the firmware's own
//! alignment and recognizer over every photo and reports that number,
//! next to the true-accept rate at three false-accept rates, which is
//! what the application actually cares about.
//!
//! The faces are found by YuNet in tract at the photo's own size, as in
//! the `crops` command, not by the firmware's 96x64 detector: an LFW face
//! fills about 40 percent of the photo's height, and the board's detector
//! is built for a face that fills 90 percent of a 60-pixel frame. What is
//! measured here is therefore the alignment and the recognizer. The
//! detector is measured separately, by `quantize`'s frame-by-frame
//! comparison and by the fixture tests.

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};

use crate::{
    data,
    embed::{Embeddings, Pick, cosine, embed_all},
    tensors::Tensors,
};

/// One pair of the protocol.
struct Pair {
    /// The first photo.
    a: PathBuf,
    /// The second photo.
    b: PathBuf,
    /// Whether they show the same person.
    same: bool,
    /// Which of the ten folds the pair belongs to.
    fold: usize,
}

/// Measure the pipeline on `pairs` over the photos in `images`.
///
/// # Errors
///
/// When a file cannot be read, the protocol file does not parse, or the
/// networks fail.
pub fn run(
    detector: &Path,
    weights: &Path,
    integer_weights: Option<&Path>,
    images: &Path,
    pairs_file: &Path,
    limit: Option<usize>,
    pick: Pick,
) -> Result<()> {
    data::people(images)?;
    let pairs = read_pairs(pairs_file, images, limit)?;
    println!("{} pairs from {}", pairs.len(), pairs_file.display());

    let mut photos: Vec<PathBuf> = pairs
        .iter()
        .flat_map(|pair| [pair.a.clone(), pair.b.clone()])
        .collect();
    photos.sort();
    photos.dedup();
    println!("{} photos to embed", photos.len());

    let f32s = Tensors::read(weights)?;
    let i8s = integer_weights
        .map(|path| {
            Tensors::read(path).map(|mut tensors| {
                tensors.pack_for_lanes();
                tensors
            })
        })
        .transpose()?;
    let embeddings = embed_all(detector, &f32s, i8s.as_ref(), &photos, pick)?;
    let missing = photos.len() - embeddings.len();
    println!("{} photos with a face, {missing} without", embeddings.len());

    report("f32", &pairs, &embeddings, |e| &e.float);
    if i8s.is_some() {
        report("int8", &pairs, &embeddings, |e| &e.integer);
        let agreement: Vec<f32> = embeddings
            .values()
            .map(|e| cosine(&e.float, &e.integer))
            .collect();
        let worst = agreement.iter().copied().fold(f32::INFINITY, f32::min);
        let mean = agreement.iter().sum::<f32>() / agreement.len() as f32;
        println!(
            "\nembedding cosine int8 vs f32 over {} photos: worst {worst:.4}, mean {mean:.4}",
            agreement.len()
        );
    }
    Ok(())
}

/// Read the protocol file. Its first line is the number of folds and the
/// number of matched pairs per fold; then, per fold, that many matched
/// pairs (`name i j`) and as many mismatched ones (`name_a i name_b j`).
fn read_pairs(path: &Path, images: &Path, limit: Option<usize>) -> Result<Vec<Pair>> {
    let text = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let mut lines = text.lines();
    let header = lines.next().context("the protocol file is empty")?;
    let mut numbers = header.split_whitespace();
    let folds: usize = numbers.next().context("no fold count")?.parse()?;
    let per_fold: usize = numbers.next().context("no pair count")?.parse()?;

    let photo = |name: &str, index: &str| -> Result<PathBuf> {
        let number: usize = index.parse()?;
        Ok(images.join(name).join(format!("{name}_{number:04}.jpg")))
    };
    let mut pairs = Vec::with_capacity(folds * per_fold * 2);
    for fold in 0..folds {
        for _ in 0..per_fold {
            let line = lines
                .next()
                .with_context(|| format!("fold {fold}: missing a matched pair"))?;
            let fields: Vec<&str> = line.split_whitespace().collect();
            let [name, first, second] = fields[..] else {
                bail!("not a matched pair: {line:?}");
            };
            pairs.push(Pair {
                a: photo(name, first)?,
                b: photo(name, second)?,
                same: true,
                fold,
            });
        }
        for _ in 0..per_fold {
            let line = lines
                .next()
                .with_context(|| format!("fold {fold}: missing a mismatched pair"))?;
            let fields: Vec<&str> = line.split_whitespace().collect();
            let [first_name, first, second_name, second] = fields[..] else {
                bail!("not a mismatched pair: {line:?}");
            };
            pairs.push(Pair {
                a: photo(first_name, first)?,
                b: photo(second_name, second)?,
                same: false,
                fold,
            });
        }
    }
    if let Some(limit) = limit {
        pairs.truncate(limit);
    }
    Ok(pairs)
}

/// Score every pair whose two photos were embedded, then print the
/// ten-fold accuracy and the true-accept rates.
fn report(
    label: &str,
    pairs: &[Pair],
    embeddings: &HashMap<PathBuf, Embeddings>,
    pick: fn(&Embeddings) -> &[f32],
) {
    let scored: Vec<(f32, bool, usize)> = pairs
        .iter()
        .filter_map(|pair| {
            let a = embeddings.get(&pair.a)?;
            let b = embeddings.get(&pair.b)?;
            Some((cosine(pick(a), pick(b)), pair.same, pair.fold))
        })
        .collect();
    let genuine: Vec<f32> = scored
        .iter()
        .filter(|(_, same, _)| *same)
        .map(|(s, _, _)| *s)
        .collect();
    let impostor: Vec<f32> = scored
        .iter()
        .filter(|(_, same, _)| !*same)
        .map(|(s, _, _)| *s)
        .collect();

    println!("\n== {label} ==");
    println!(
        "{} pairs scored ({} same, {} different)",
        scored.len(),
        genuine.len(),
        impostor.len()
    );
    println!(
        "  same person:      mean {:.3}, spread {:.3}",
        mean(&genuine),
        spread(&genuine)
    );
    println!(
        "  different people: mean {:.3}, spread {:.3}",
        mean(&impostor),
        spread(&impostor)
    );

    let folds = scored.iter().map(|(_, _, fold)| *fold).max().unwrap_or(0) + 1;
    let mut accuracies = Vec::new();
    let mut thresholds = Vec::new();
    for fold in 0..folds {
        let train: Vec<(f32, bool)> = scored
            .iter()
            .filter(|(_, _, f)| *f != fold)
            .map(|(s, same, _)| (*s, *same))
            .collect();
        let test: Vec<(f32, bool)> = scored
            .iter()
            .filter(|(_, _, f)| *f == fold)
            .map(|(s, same, _)| (*s, *same))
            .collect();
        if train.is_empty() || test.is_empty() {
            continue;
        }
        let threshold = best_threshold(&train);
        accuracies.push(accuracy(&test, threshold));
        thresholds.push(threshold);
    }
    println!(
        "  ten-fold accuracy: {:.2} % +- {:.2} (threshold {:.3} +- {:.3})",
        100.0 * mean(&accuracies),
        100.0 * spread(&accuracies),
        mean(&thresholds),
        spread(&thresholds)
    );

    let mut sorted_impostor = impostor.clone();
    sorted_impostor.sort_by(f32::total_cmp);
    for far in [1e-2f32, 1e-3, 1e-4] {
        // The threshold that lets through `far` of the impostors.
        let index = ((1.0 - far) * sorted_impostor.len() as f32).ceil() as usize;
        let threshold = sorted_impostor
            .get(index.min(sorted_impostor.len() - 1))
            .copied()
            .unwrap_or(1.0);
        let accepted = genuine.iter().filter(|&&s| s >= threshold).count();
        let tar = accepted as f32 / genuine.len().max(1) as f32;
        println!(
            "  TAR at FAR {far:.0e}: {:.2} % (threshold {threshold:.3})",
            100.0 * tar
        );
    }
}

/// The mean of a list, or 0 when it is empty.
fn mean(values: &[f32]) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    values.iter().sum::<f32>() / values.len() as f32
}

/// The standard deviation of a list.
fn spread(values: &[f32]) -> f32 {
    if values.len() < 2 {
        return 0.0;
    }
    let mean = mean(values);
    (values.iter().map(|v| (v - mean) * (v - mean)).sum::<f32>() / values.len() as f32).sqrt()
}

/// The fraction of pairs that `threshold` decides correctly.
fn accuracy(pairs: &[(f32, bool)], threshold: f32) -> f32 {
    let correct = pairs
        .iter()
        .filter(|(score, same)| (*score >= threshold) == *same)
        .count();
    correct as f32 / pairs.len().max(1) as f32
}

/// The threshold with the best accuracy on `pairs`: every score is tried.
fn best_threshold(pairs: &[(f32, bool)]) -> f32 {
    let mut candidates: Vec<f32> = pairs.iter().map(|(score, _)| *score).collect();
    candidates.sort_by(f32::total_cmp);
    candidates.dedup();
    candidates
        .iter()
        .copied()
        .max_by(|&a, &b| accuracy(pairs, a).total_cmp(&accuracy(pairs, b)))
        .unwrap_or(0.0)
}
