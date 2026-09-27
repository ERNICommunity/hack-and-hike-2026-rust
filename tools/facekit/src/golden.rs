//! The `golden` command: reference inputs and outputs from tract, for the
//! tests of the firmware's own implementation.
//!
//! A golden file holds the 8-bit image that went in, the `f32` input tensor
//! as the model saw it, the output of every block boundary of the network,
//! and the final outputs. The firmware tests run their kernels on the same
//! input and compare block by block, so the first block that differs names
//! the broken kernel.

use std::{fs, path::Path};

use anyhow::{Context, Result, bail};
use clap::ValueEnum;
use sha2::{Digest, Sha256};
use tract_onnx::pb::GraphProto;

use crate::{
    blob::Writer,
    names,
    onnx::{self, Runner},
    synthetic,
};

/// Which model the golden file is for. Each has its own input size,
/// preprocessing and block boundaries.
#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum Preset {
    /// EdgeFace-XXS: 112x112 RGB, normalized to -1..1.
    Edgeface,
    /// YuNet: 64x96 BGR, 0..255, with the 80x60 image in the top-left
    /// corner and black padding, as on the board.
    Yunet,
}

/// Where the input image comes from.
#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum Source {
    /// The drawn face of `synthetic::face`.
    Face,
    /// Uniform noise.
    Noise,
}

/// Write the golden file `out` for `preset`, from `image` (a photo) or
/// `source` (a synthetic image) with `seed`.
///
/// # Errors
///
/// When the model or image cannot be read, tract fails, or two runs give
/// different results.
pub fn run(
    preset: Preset,
    model: &Path,
    image: Option<&Path>,
    source: Source,
    seed: u64,
    out: &Path,
) -> Result<()> {
    let (height, width) = match preset {
        Preset::Edgeface => (112, 96 + 16),
        Preset::Yunet => (64, 96),
    };
    let rgb = match image {
        Some(path) => load_image(path, preset, width, height)?,
        None => match source {
            Source::Face => synthetic::face(width, height),
            Source::Noise => synthetic::noise(width, height, seed),
        },
    };
    let input_shape = [1, 3, height, width];
    let input = preprocess(preset, &rgb, width, height);

    let proto = onnx::load_proto(model)?;
    let graph = onnx::graph(&proto)?;
    let (tensor_names, entry_names) = boundaries(preset, graph);
    let runner = Runner::new(model, &input_shape, &tensor_names)?;
    let outputs = runner.run(&input_shape, &input)?;
    let again = runner.run(&input_shape, &input)?;
    if outputs != again {
        bail!("tract gave different outputs for the same input");
    }

    let mut writer = Writer::new();
    writer.add_u8("image_rgb", "HWC", &[height, width, 3], &rgb)?;
    writer.add_f32("input", "NCHW", &input_shape, &input)?;
    for output in &outputs {
        let index = tensor_names
            .iter()
            .position(|name| *name == output.name)
            .context("unknown output")?;
        let (shape, values) = (&output.shape, &output.values);
        // LayerNormalization works on channels-last data in these graphs.
        let layout = match shape.len() {
            4 if output.name.contains("LayerNormalization") => "NHWC",
            4 => "NCHW",
            3 => "NAK",
            2 => "NC",
            _ => "RAW",
        };
        writer.add_f32(&entry_names[index], layout, shape, values)?;
    }
    let manifest = writer.manifest();
    let bytes = writer.finish();
    fs::write(out, &bytes).with_context(|| format!("writing {}", out.display()))?;
    print!("{manifest}");
    println!(
        "wrote {} ({} bytes, sha256 {:x})",
        out.display(),
        bytes.len(),
        Sha256::digest(&bytes)
    );
    Ok(())
}

/// A photo as RGB bytes of the preset's input size. EdgeFace gets the
/// whole photo scaled to 112x112. YuNet gets it scaled to 80x60 in the
/// top-left corner of a black 96x64 image, which is what the board does
/// with its 320x240 frame.
fn load_image(path: &Path, preset: Preset, width: usize, height: usize) -> Result<Vec<u8>> {
    use image::imageops::FilterType;
    let photo = image::open(path)
        .with_context(|| format!("reading {}", path.display()))?
        .to_rgb8();
    let (content_width, content_height) = match preset {
        Preset::Edgeface => (width, height),
        Preset::Yunet => (80, 60),
    };
    let scaled = image::imageops::resize(
        &photo,
        content_width as u32,
        content_height as u32,
        FilterType::Triangle,
    );
    let mut rgb = vec![0u8; width * height * 3];
    for y in 0..content_height {
        let source = &scaled.as_raw()[y * content_width * 3..(y + 1) * content_width * 3];
        rgb[y * width * 3..y * width * 3 + content_width * 3].copy_from_slice(source);
    }
    Ok(rgb)
}

/// The `f32` input tensor, `[1][3][H][W]`, from RGB bytes.
fn preprocess(preset: Preset, rgb: &[u8], width: usize, height: usize) -> Vec<f32> {
    let mut input = vec![0.0f32; 3 * width * height];
    for y in 0..height {
        for x in 0..width {
            let pixel = &rgb[(y * width + x) * 3..(y * width + x) * 3 + 3];
            for (channel, &byte) in pixel.iter().enumerate() {
                let (plane, value) = match preset {
                    // RGB planes, -1..1.
                    Preset::Edgeface => (channel, (f32::from(byte) / 255.0 - 0.5) / 0.5),
                    // BGR planes, 0..255.
                    Preset::Yunet => (2 - channel, f32::from(byte)),
                };
                input[(plane * height + y) * width + x] = value;
            }
        }
    }
    input
}

/// The nodes whose outputs the golden file records: the ONNX tensor names
/// tract needs, and the short names the file uses, in the same order.
fn boundaries(preset: Preset, graph: &GraphProto) -> (Vec<String>, Vec<String>) {
    let mut tensor_names = Vec::new();
    let mut entry_names = Vec::new();
    for node in &graph.node {
        let segments = names::segments(&node.name);
        let wanted = match preset {
            Preset::Edgeface => {
                node.name == "/model/stem/stem.1/Transpose_1"
                    || (node.op_type == "Add"
                        && segments.len() == 3
                        && segments[0].starts_with("stages.")
                        && segments[1].starts_with("blocks."))
                    || (node.op_type == "Conv" && node.name.contains("/downsample/"))
                    || node.op_type == "GlobalAveragePool"
                    || node.name == "/model/head/norm/LayerNormalization"
            }
            Preset::Yunet => matches!(node.op_type.as_str(), "MaxPool" | "Add" | "Relu" | "Resize"),
        };
        if wanted {
            tensor_names.push(node.output[0].clone());
            entry_names.push(names::node(&node.name));
        }
    }
    for output in &graph.output {
        tensor_names.push(output.name.clone());
        entry_names.push(output.name.clone());
    }
    (tensor_names, entry_names)
}
