//! The `calibrate` command: where to put the two thresholds.
//!
//! The application accepts a face when it scores at least
//! `Thresholds::accept` against the enrolled person's templates *and*
//! beats the closest stranger in the impostor bank by at least
//! `Thresholds::margin`. This command measures what those two numbers
//! cost and buy, by playing the application many times over:
//!
//! - pick a person with enough photos, enroll some of them as templates,
//! - the person's remaining photos are the genuine attempts,
//! - one photo each of many other people are the impostor attempts,
//! - score every attempt against that person's templates and against the
//!   bank, exactly as `gallery::Gallery::match_probe` does,
//! - repeat for many people and put all the attempts together.
//!
//! The report is the trade-off: for each false-accept rate, the highest
//! true-accept rate and the thresholds that reach it, once for a single
//! frame and once for three frames averaged (`gallery::Fusion`), which is
//! what the application does.
//!
//! LFW photos of one person come from different occasions, so this is a
//! harder test than enrolling and recognizing in one sitting in front of
//! the board: the thresholds it suggests are on the safe side.

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use hack_and_hike_vision::{
    blob::Blob,
    gallery::{Embedding, Fusion, Gallery, ImpostorBank, MAX_TEMPLATES, Thresholds},
};

use crate::{
    data,
    embed::{Embeddings, Pick, embed_all},
    tensors::Tensors,
};

/// The accept thresholds that are tried.
const ACCEPT_STEPS: usize = 61;
/// The smallest and largest accept threshold.
const ACCEPT_RANGE: (f32, f32) = (0.10, 0.70);
/// The margins that are tried.
const MARGIN_STEPS: usize = 33;
/// The smallest and largest margin. Negative margins are in the grid on
/// purpose: a margin of -0.2 means "the person may score up to 0.2 below
/// the closest stranger and still be accepted", which all but switches
/// the rule off. Letting the sweep reach for that is how the measurement
/// can say whether the rule earns its place.
const MARGIN_RANGE: (f32, f32) = (-0.32, 0.32);
/// The false-accept rates the report picks operating points for. The
/// last one is "no impostor at all got in".
const TARGET_RATES: [f32; 4] = [1e-2, 3e-3, 1e-3, 0.0];

/// The embedding the board would use: the integer one when integer
/// weights were given, otherwise the `f32` one.
fn pick(embeddings: &Embeddings) -> &[f32] {
    if embeddings.integer.is_empty() {
        &embeddings.float
    } else {
        &embeddings.integer
    }
}

/// What an attempt scored: against the enrolled person's templates,
/// against the closest stranger, and whether it really was the person.
#[derive(Clone, Copy)]
struct Attempt {
    /// The best similarity over the person's templates.
    score: f32,
    /// That score minus the closest stranger's.
    margin: f32,
    /// Whether the probe really is the enrolled person.
    genuine: bool,
}

impl Attempt {
    /// Whether `thresholds` accept this attempt. The same rule as
    /// `Gallery::match_probe`; `check_against_the_firmware` proves it.
    fn accepted(&self, accept: f32, margin: f32) -> bool {
        self.score >= accept && self.margin >= margin
    }
}

/// Measure the decision over `images` and print the trade-off.
///
/// # Errors
///
/// When a folder cannot be read, the bank cannot be parsed, or too few
/// people have enough photos.
#[allow(clippy::too_many_arguments)]
pub fn run(
    detector: &Path,
    weights: &Path,
    integer_weights: Option<&Path>,
    images: &Path,
    bank_file: &Path,
    templates: usize,
    people_count: usize,
    impostors: usize,
) -> Result<()> {
    let bank_bytes =
        fs::read(bank_file).with_context(|| format!("reading {}", bank_file.display()))?;
    let blob = Blob::parse(&bank_bytes)
        .map_err(|error| anyhow::anyhow!("{}: {error:?}", bank_file.display()))?;
    let entry = blob
        .get("impostors")
        .context("the bank has no `impostors` tensor")?;
    let bank = ImpostorBank::from_i8(entry.i8_slice(), entry.scale);
    println!("impostor bank: {} strangers", bank.len());

    let people = people_with_photos(images, templates)?;
    if people.len() < people_count {
        bail!(
            "only {} people have more than {templates} photos, need {people_count}",
            people.len()
        );
    }
    // Spread the choice over the listing rather than taking the first N.
    let step = people.len() / people_count;
    let chosen: Vec<&(PathBuf, Vec<PathBuf>)> = (0..people_count)
        .map(|index| &people[index * step])
        .collect();
    // One photo each of the people who were not chosen: the strangers
    // who walk up to the device.
    let strangers: Vec<PathBuf> = people
        .iter()
        .filter(|(folder, _)| !chosen.iter().any(|(picked, _)| picked == folder))
        .map(|(_, shots)| shots[0].clone())
        .take(impostors)
        .collect();
    println!(
        "{} people have more than {templates} photos; {} enrolled, {} strangers",
        people.len(),
        chosen.len(),
        strangers.len()
    );

    let mut needed: Vec<PathBuf> = chosen.iter().flat_map(|(_, shots)| shots.clone()).collect();
    needed.extend(strangers.iter().cloned());
    needed.sort();
    needed.dedup();
    println!("{} photos to embed", needed.len());

    let f32s = Tensors::read(weights)?;
    let i8s = integer_weights
        .map(|path| {
            Tensors::read(path).map(|mut tensors| {
                tensors.pack_for_lanes();
                tensors
            })
        })
        .transpose()?;
    let embedded = embed_all(detector, &f32s, i8s.as_ref(), &needed, Pick::Centre)?;
    let vector = |path: &PathBuf| embedded.get(path).map(|e| Embedding::from_raw(pick(e)));
    let stranger_probes: Vec<Embedding> = strangers.iter().filter_map(vector).collect();

    let mut single = Vec::new();
    let mut fused = Vec::new();
    let mut enrollments = 0;
    for (folder, shots) in &chosen {
        let mut person_templates: Vec<Embedding> =
            shots[..templates].iter().filter_map(vector).collect();
        person_templates.truncate(MAX_TEMPLATES);
        if person_templates.is_empty() {
            continue;
        }
        enrollments += 1;
        let best = |probe: &Embedding| {
            person_templates
                .iter()
                .map(|template| template.similarity(probe))
                .fold(-1.0f32, f32::max)
        };
        let genuine_probes: Vec<Embedding> = shots[templates..].iter().filter_map(vector).collect();
        for (probes, genuine) in [(&genuine_probes, true), (&stranger_probes, false)] {
            for probe in probes {
                let score = best(probe);
                single.push(Attempt {
                    score,
                    margin: score - bank.best_similarity(probe),
                    genuine,
                });
            }
            // Three frames averaged, as the application does. For a
            // stranger the three frames are three different people,
            // which is the worst case for the average.
            for window in probes.chunks(3).filter(|window| window.len() == 3) {
                let mut fusion = Fusion::new();
                for probe in window {
                    fusion.push(*probe);
                }
                if let Some(probe) = fusion.fused() {
                    let score = best(&probe);
                    fused.push(Attempt {
                        score,
                        margin: score - bank.best_similarity(&probe),
                        genuine,
                    });
                }
            }
        }
        let _ = folder;
    }
    println!(
        "{enrollments} enrollments: {} single-frame attempts ({} genuine), {} three-frame attempts ({} genuine)",
        single.len(),
        single.iter().filter(|a| a.genuine).count(),
        fused.len(),
        fused.iter().filter(|a| a.genuine).count()
    );
    check_against_the_firmware(&chosen, &embedded, templates, &bank, &stranger_probes)?;

    for (label, attempts) in [("one frame", &single), ("three frames averaged", &fused)] {
        report(label, attempts);
    }
    Ok(())
}

/// The people of `images` with more than `templates` photos, sorted.
fn people_with_photos(images: &Path, templates: usize) -> Result<Vec<(PathBuf, Vec<PathBuf>)>> {
    let folders = data::people(images)?;
    let mut people = Vec::new();
    for folder in folders {
        let mut shots: Vec<PathBuf> = fs::read_dir(&folder)?
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| {
                path.extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("jpg"))
            })
            .collect();
        shots.sort();
        if shots.len() > templates {
            people.push((folder, shots));
        }
    }
    Ok(people)
}

/// Prove that the sweep's rule is the firmware's rule: build one real
/// [`Gallery`], run [`Gallery::match_probe`] over a few thresholds, and
/// compare its verdict with [`Attempt::accepted`].
///
/// # Errors
///
/// When no person could be enrolled, or the two disagree.
fn check_against_the_firmware(
    chosen: &[&(PathBuf, Vec<PathBuf>)],
    embedded: &std::collections::HashMap<PathBuf, Embeddings>,
    templates: usize,
    bank: &ImpostorBank<'_>,
    strangers: &[Embedding],
) -> Result<()> {
    let Some((folder, shots)) = chosen.first().map(|entry| (&entry.0, &entry.1)) else {
        bail!("nobody to check against");
    };
    let mut gallery = Gallery::new();
    let name = folder
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("person");
    let person = gallery
        .enroll(&name.chars().take(16).collect::<String>())
        .context("the gallery refused the name")?;
    for shot in shots.iter().take(templates.min(MAX_TEMPLATES)) {
        if let Some(embedding) = embedded.get(shot).map(|e| Embedding::from_raw(pick(e))) {
            person.add_template(embedding);
        }
    }
    let probes: Vec<Embedding> = shots[templates..]
        .iter()
        .filter_map(|shot| embedded.get(shot).map(|e| Embedding::from_raw(pick(e))))
        .chain(strangers.iter().copied())
        .collect();
    let mut checked = 0;
    for accept in [0.15f32, 0.3, 0.45, 0.6] {
        for margin in [0.0f32, 0.1, 0.25] {
            let thresholds = Thresholds { accept, margin };
            for probe in &probes {
                let verdict = gallery.match_probe(probe, bank, &thresholds);
                let score = gallery.people()[0].best_similarity(probe);
                let attempt = Attempt {
                    score,
                    margin: score - bank.best_similarity(probe),
                    genuine: true,
                };
                if verdict.is_known() != attempt.accepted(accept, margin) {
                    bail!(
                        "the sweep and `Gallery::match_probe` disagree at accept {accept}, margin {margin}"
                    );
                }
                checked += 1;
            }
        }
    }
    println!("the sweep's rule matches `Gallery::match_probe` on {checked} decisions\n");
    Ok(())
}

/// Print the trade-off for one set of attempts.
fn report(label: &str, attempts: &[Attempt]) {
    let genuine = attempts.iter().filter(|a| a.genuine).count();
    let impostor = attempts.len() - genuine;
    if genuine == 0 || impostor == 0 {
        return;
    }
    println!("== {label} ==");
    println!("  {genuine} genuine attempts, {impostor} impostor attempts");

    for target in TARGET_RATES {
        let mut best: Option<(f32, f32, f32, f32)> = None;
        for accept_step in 0..ACCEPT_STEPS {
            let accept = ACCEPT_RANGE.0
                + (ACCEPT_RANGE.1 - ACCEPT_RANGE.0) * accept_step as f32
                    / (ACCEPT_STEPS - 1) as f32;
            for margin_step in 0..MARGIN_STEPS {
                let margin = MARGIN_RANGE.0
                    + (MARGIN_RANGE.1 - MARGIN_RANGE.0) * margin_step as f32
                        / (MARGIN_STEPS - 1) as f32;
                let false_accepts = attempts
                    .iter()
                    .filter(|a| !a.genuine && a.accepted(accept, margin))
                    .count();
                let far = false_accepts as f32 / impostor as f32;
                if far > target {
                    continue;
                }
                let true_accepts = attempts
                    .iter()
                    .filter(|a| a.genuine && a.accepted(accept, margin))
                    .count();
                let tar = true_accepts as f32 / genuine as f32;
                if best.is_none_or(|(previous, _, _, _)| tar > previous) {
                    best = Some((tar, far, accept, margin));
                }
            }
        }
        match best {
            Some((tar, far, accept, margin)) => println!(
                "  at most {:>6} false accepts: {:>6.2} % recognized (accept {accept:.3}, margin {margin:.3}, measured {:.4} %)",
                format!("{:.1}%", 100.0 * target),
                100.0 * tar,
                100.0 * far
            ),
            None => println!(
                "  at most {:.1}% false accepts: unreachable",
                100.0 * target
            ),
        }
    }
    // A few points worth naming, so that the choice of a default can be
    // read off the measurement instead of the sweep's optimum, which
    // sits exactly on the boundary and would be fragile.
    println!("  candidate operating points:");
    for (accept, margin) in [
        (0.30f32, 0.0f32),
        (0.35, 0.0),
        (0.40, 0.0),
        (0.45, 0.0),
        (0.35, 0.10),
        (0.40, 0.10),
    ] {
        let tar = attempts
            .iter()
            .filter(|a| a.genuine && a.accepted(accept, margin))
            .count() as f32
            / genuine as f32;
        let far = attempts
            .iter()
            .filter(|a| !a.genuine && a.accepted(accept, margin))
            .count() as f32
            / impostor as f32;
        println!(
            "    accept {accept:.2}, margin {margin:.2}: {:>6.2} % recognized, {:.3} % false accepts",
            100.0 * tar,
            100.0 * far
        );
    }

    // What the margin is worth: the best accept-only threshold at the
    // strictest rate, for comparison. A margin of -2 is never binding.
    let strictest = TARGET_RATES[TARGET_RATES.len() - 1];
    let mut without_margin = 0.0f32;
    for accept_step in 0..ACCEPT_STEPS {
        let accept = ACCEPT_RANGE.0
            + (ACCEPT_RANGE.1 - ACCEPT_RANGE.0) * accept_step as f32 / (ACCEPT_STEPS - 1) as f32;
        let far = attempts
            .iter()
            .filter(|a| !a.genuine && a.accepted(accept, -2.0))
            .count() as f32
            / impostor as f32;
        if far > strictest {
            continue;
        }
        let tar = attempts
            .iter()
            .filter(|a| a.genuine && a.accepted(accept, -2.0))
            .count() as f32
            / genuine as f32;
        without_margin = without_margin.max(tar);
    }
    println!(
        "  the same without the margin rule:  {:>6.2} % recognized\n",
        100.0 * without_margin
    );
}
