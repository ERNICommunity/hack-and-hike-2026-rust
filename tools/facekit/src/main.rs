//! facekit: the developer-machine side of the face ID app.
//!
//! It runs on your computer, never on the board. Step by step it grows the
//! commands that turn the downloaded ONNX models into data the firmware can
//! use, and that measure how well the pipeline works.
//!
//! ```text
//! cargo run --release -- inspect --input-shape 1,3,112,112 models/edgeface_xxs.onnx
//! cargo run --release -- export --input-shape 1,3,112,112 models/edgeface_xxs.onnx out.fkb
//! cargo run --release -- golden edgeface models/edgeface_xxs.onnx out.fkb
//! ```

mod bank;
mod blob;
mod calibrate;
mod crops;
mod data;
mod embed;
mod eval;
mod export;
mod golden;
mod inspect;
mod names;
mod onnx;
mod quantize;
mod recognizer;
mod synthetic;
mod tensors;

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};

/// The command line of facekit.
#[derive(Parser)]
#[command(version, about)]
struct Cli {
    /// The command to run.
    #[command(subcommand)]
    command: Command,
}

/// The commands of facekit. Later steps add more.
#[derive(Subcommand)]
enum Command {
    /// List the operators, tensors and sizes of an ONNX model, and check that
    /// every operator is one the firmware implements.
    Inspect {
        /// The ONNX file.
        model: PathBuf,
        /// Fail when an operator is not in the allowed list. Prints the
        /// unknown operators either way.
        #[arg(long)]
        strict: bool,
        /// Fix the input shape, for models whose input has symbolic
        /// dimensions such as `batch_size` or `height`. Comma separated,
        /// for example `1,3,112,112`.
        #[arg(long, value_delimiter = ',')]
        input_shape: Option<Vec<usize>>,
    },
    /// Write the weights of a model as an FKB1 file for the firmware, plus
    /// a `.txt` listing next to it.
    Export {
        /// The ONNX file.
        model: PathBuf,
        /// The FKB1 file to write.
        out: PathBuf,
        /// The input shape, comma separated, for example `1,3,112,112`.
        /// Needed to evaluate the constants that depend on the input size.
        #[arg(long, value_delimiter = ',')]
        input_shape: Vec<usize>,
    },
    /// Write reference inputs and outputs of a model as an FKB1 file, for
    /// the tests of the firmware's implementation.
    Golden {
        /// Which model, with its input size and preprocessing.
        preset: golden::Preset,
        /// The ONNX file.
        model: PathBuf,
        /// The FKB1 file to write.
        out: PathBuf,
        /// A photo to use as input (PNG or JPEG). Without it, a synthetic
        /// image is used.
        #[arg(long)]
        image: Option<PathBuf>,
        /// Which synthetic image to use when no photo is given.
        #[arg(long, value_enum, default_value_t = golden::Source::Face)]
        source: golden::Source,
        /// The seed of the synthetic noise.
        #[arg(long, default_value_t = 1)]
        seed: u64,
    },
    /// Find the faces in a folder of photos and write, for each, the
    /// aligned 112x112 crop and a 320x240 frame with the face filling the
    /// height: calibration and evaluation data.
    Crops {
        /// The YuNet ONNX file (the dynamic-size variant).
        model: PathBuf,
        /// The folder of photos.
        photos: PathBuf,
        /// The folder to write `crops/` and `frames/` into.
        out: PathBuf,
    },
    /// Measure the pipeline on a pair protocol such as Labeled Faces in
    /// the Wild: ten-fold accuracy and true-accept rates.
    Eval {
        /// The YuNet ONNX file (the dynamic-size variant), to find the
        /// faces in the photos.
        detector: PathBuf,
        /// The `f32` FKB1 weights of the recognizer.
        weights: PathBuf,
        /// The folder with one subfolder of photos per person.
        images: PathBuf,
        /// The protocol file (`pairs.txt`).
        pairs: PathBuf,
        /// Also measure the integer recognizer with these weights.
        #[arg(long)]
        int8: Option<PathBuf>,
        /// Use at most this many pairs.
        #[arg(long)]
        limit: Option<usize>,
        /// Which face to measure when a photo holds more than one.
        #[arg(long, value_enum, default_value_t = embed::Pick::Score)]
        pick: embed::Pick,
    },
    /// Write the impostor bank: embeddings of people the device does not
    /// know, so that recognition can ask "more like you than like a
    /// stranger?" instead of trusting a fixed threshold.
    Bank {
        /// The YuNet ONNX file (the dynamic-size variant).
        detector: PathBuf,
        /// The `f32` FKB1 weights of the recognizer.
        weights: PathBuf,
        /// The folder with one subfolder of photos per person.
        images: PathBuf,
        /// The FKB1 file to write.
        out: PathBuf,
        /// The integer weights; when given, the bank holds what the
        /// integer recognizer produces, which is what the board runs.
        #[arg(long)]
        int8: Option<PathBuf>,
        /// How many people to put in the bank.
        #[arg(long, default_value_t = 200)]
        count: usize,
    },
    /// Measure where to put the two thresholds of the decision and the
    /// steps above them from which a decision is sure, by playing the
    /// application over a folder of labelled photos.
    Calibrate {
        /// The YuNet ONNX file (the dynamic-size variant).
        detector: PathBuf,
        /// The `f32` FKB1 weights of the recognizer.
        weights: PathBuf,
        /// The folder with one subfolder of photos per person.
        images: PathBuf,
        /// The impostor bank from `bank`.
        bank: PathBuf,
        /// The integer weights; when given, the integer recognizer is
        /// measured, which is what the board runs.
        #[arg(long)]
        int8: Option<PathBuf>,
        /// How many photos to enroll per person.
        #[arg(long, default_value_t = 5)]
        templates: usize,
        /// How many people to enroll, one after the other.
        #[arg(long, default_value_t = 120)]
        people: usize,
        /// How many strangers try to be recognized as each of them.
        #[arg(long, default_value_t = 400)]
        impostors: usize,
    },
    /// Quantize a network's `f32` weights with a calibration set, write
    /// the integer FKB1 file, and report the accuracy cost.
    Quantize {
        /// Which network.
        model: quantize::Model,
        /// The `f32` FKB1 file from `export`.
        weights: PathBuf,
        /// A folder of calibration images: 112x112 crops for edgeface,
        /// 320x240 frames for yunet.
        samples: PathBuf,
        /// The integer FKB1 file to write.
        out: PathBuf,
        /// Use at most this many samples.
        #[arg(long)]
        limit: Option<usize>,
    },
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Inspect {
            model,
            strict,
            input_shape,
        } => inspect::run(&model, strict, input_shape.as_deref()),
        Command::Export {
            model,
            out,
            input_shape,
        } => export::run(&model, &input_shape, &out),
        Command::Golden {
            preset,
            model,
            out,
            image,
            source,
            seed,
        } => golden::run(preset, &model, image.as_deref(), source, seed, &out),
        Command::Crops { model, photos, out } => crops::run(&model, &photos, &out),
        Command::Eval {
            detector,
            weights,
            images,
            pairs,
            int8,
            limit,
            pick,
        } => eval::run(
            &detector,
            &weights,
            int8.as_deref(),
            &images,
            &pairs,
            limit,
            pick,
        ),
        Command::Bank {
            detector,
            weights,
            images,
            out,
            int8,
            count,
        } => bank::run(&detector, &weights, int8.as_deref(), &images, &out, count),
        Command::Calibrate {
            detector,
            weights,
            images,
            bank,
            int8,
            templates,
            people,
            impostors,
        } => calibrate::run(
            &detector,
            &weights,
            int8.as_deref(),
            &images,
            &bank,
            templates,
            people,
            impostors,
        ),
        Command::Quantize {
            model,
            weights,
            samples,
            out,
            limit,
        } => quantize::run(model, &weights, &samples, &out, limit),
    }
}
