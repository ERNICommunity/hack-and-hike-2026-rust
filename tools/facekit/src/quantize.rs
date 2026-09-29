//! The `quantize` command: from the `f32` weights and a calibration set to
//! the integer weights the board runs, with a report of what the integers
//! cost in accuracy.
//!
//! 1. Run the `f32` network on every calibration sample with its probes
//!    on: first for the range of every probed tensor (per channel where
//!    the network asks for it), then for a histogram of its values.
//! 2. Choose each tensor's mapping by the clipping range with the least
//!    mean squared error over the histogram: a heavy-tailed tensor gives
//!    up its few extreme values for a finer step on all the others.
//! 3. Fold every LayerNorm's scale and shift into the weights of the layer
//!    after it (exact in `f32`), so the quantized tensor is the
//!    standardized one; quantize every weight per output channel.
//! 4. Write the file, read it back, and run the integer network next to
//!    the `f32` one on every sample: signal-to-noise per block, and what
//!    the application sees (embedding similarity, or box and landmark
//!    positions).

use std::{collections::BTreeMap, fs, path::Path};

use anyhow::{Context, Result, bail};
use clap::ValueEnum;
use hack_and_hike_vision::{
    align::{CROP_SIZE, recognizer_input, recognizer_input_i8},
    blob::DataType,
    detect::{
        CONTENT_HEIGHT, CONTENT_WIDTH, DEFAULT_NMS_THRESHOLD, DEFAULT_SCORE_THRESHOLD, DOWNSCALE,
        decode, detector_input, detector_input_i8,
    },
    image::{Rgb565Frame, RgbImage, RgbImageMut, downscale_to_rgb},
    nn::{
        Shape, edgeface,
        lanes::GroupPlan,
        quant::{Granularity, Quant, quantize_weight_channels, quantize_weight_rows, snr_db},
        yunet,
    },
};
use sha2::{Digest, Sha256};

use crate::{
    blob::{Tensor, Writer},
    data,
    recognizer::Runner,
    tensors::Tensors,
};

/// Which network to quantize.
#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum Model {
    /// The recognizer; samples are 112x112 aligned crops.
    Edgeface,
    /// The detector; samples are 320x240 frames.
    Yunet,
}

/// The chosen mapping of every probed tensor: one `[scale, zero point]`
/// pair for a per-tensor mapping, one per channel otherwise, and whether
/// the tensor is 16-bit.
type Mappings = BTreeMap<String, (Vec<Quant>, bool)>;

/// Smallest and largest value seen, per probe name, per channel (one
/// entry for a per-tensor probe), and whether the tensor is 16-bit.
type Ranges = BTreeMap<String, (Vec<(f32, f32)>, bool)>;

/// Bins of the histogram of every probed tensor.
const BINS: usize = 2048;
/// Candidate clip positions per side of the range.
const CLIP_STEPS: usize = 96;

/// The value histograms of the probed tensors, per channel, over their
/// calibrated ranges.
struct Histograms {
    /// The ranges the bins span.
    ranges: Ranges,
    /// Per name, per channel, `BINS` counts.
    counts: BTreeMap<String, Vec<Vec<u32>>>,
}

impl Histograms {
    /// Empty histograms over `ranges`.
    fn new(ranges: Ranges) -> Self {
        let counts = ranges
            .iter()
            .map(|(name, (channels, _))| (name.clone(), vec![vec![0u32; BINS]; channels.len()]))
            .collect();
        Self { ranges, counts }
    }

    /// Count the values of a probed tensor.
    fn observe(&mut self, name: &str, granularity: Granularity, values: &[f32]) {
        let (ranges, wide) = &self.ranges[name];
        if *wide {
            // 16-bit tensors keep their full range: no histogram needed.
            return;
        }
        let counts = self.counts.get_mut(name).expect("histogram");
        let channels = granularity.channels();
        for (index, &value) in values.iter().enumerate() {
            let channel = index % channels;
            let (min, max) = ranges[channel];
            let position = if max > min {
                ((value - min) / (max - min) * BINS as f32) as usize
            } else {
                0
            };
            counts[channel][position.min(BINS - 1)] += 1;
        }
    }

    /// The mapping of every tensor and channel with the least
    /// quantization error over its histogram.
    fn mappings(&self) -> Mappings {
        self.ranges
            .iter()
            .map(|(name, (channels, wide))| {
                let quants = channels
                    .iter()
                    .zip(&self.counts[name])
                    .map(|(&(min, max), counts)| {
                        if *wide {
                            Quant::from_range16(min, max)
                        } else {
                            best_clip(min, max, counts)
                        }
                    })
                    .collect();
                (name.clone(), (quants, *wide))
            })
            .collect()
    }
}

/// The mapping over `[min, max]` (widened to include 0) with the least
/// mean squared error on a histogram: values inside the clip range get
/// the uniform rounding error of the step, values outside get their
/// distance to the clip. The two clips are chosen one after the other,
/// twice.
fn best_clip(min: f32, max: f32, counts: &[u32]) -> Quant {
    let (min, max) = (min.min(0.0), max.max(0.0));
    if max <= min {
        return Quant::from_range(min, max);
    }
    let width = (max - min) / counts.len() as f32;
    let centres: Vec<f32> = (0..counts.len())
        .map(|bin| min + (bin as f32 + 0.5) * width)
        .collect();
    let error = |low: f32, high: f32| -> f64 {
        let step = (high - low) / 255.0;
        let inside = f64::from(step * step / 12.0);
        centres
            .iter()
            .zip(counts)
            .map(|(&value, &count)| {
                let e = if value < low {
                    f64::from((low - value) * (low - value))
                } else if value > high {
                    f64::from((value - high) * (value - high))
                } else {
                    inside
                };
                e * f64::from(count)
            })
            .sum()
    };
    let (mut low, mut high) = (min, max);
    for _ in 0..2 {
        // The high clip: from a quarter of the range up to all of it; the
        // low clip likewise, on its own side. 0 always stays inside.
        let candidates = |from: f32, to: f32| {
            (0..=CLIP_STEPS).map(move |k| from + (to - from) * k as f32 / CLIP_STEPS as f32)
        };
        high = candidates(max * 0.25, max)
            .filter(|&h| h >= 0.0)
            .min_by(|&a, &b| error(low, a).total_cmp(&error(low, b)))
            .unwrap_or(max);
        low = candidates(min * 0.25, min)
            .filter(|&l| l <= 0.0)
            .min_by(|&a, &b| error(a, high).total_cmp(&error(b, high)))
            .unwrap_or(min);
    }
    Quant::from_range(low, high)
}

/// Quantize `weights` (an `f32` FKB1 file) with the images in `samples`,
/// write `out`, and report.
///
/// # Errors
///
/// When files cannot be read or written, no samples are found, or the
/// networks fail.
pub fn run(
    model: Model,
    weights: &Path,
    samples: &Path,
    out: &Path,
    limit: Option<usize>,
) -> Result<()> {
    let f32s = Tensors::read(weights)?;
    let inputs = load_samples(model, samples, limit)?;
    if inputs.is_empty() {
        bail!("no samples in {}", samples.display());
    }
    println!("{} calibration samples", inputs.len());

    let ranges = calibrate(model, &f32s, &inputs);
    let mut histograms = Histograms::new(ranges);
    run_probed(model, &f32s, &inputs, |name, granularity, values| {
        histograms.observe(name, granularity, values)
    });
    let mappings = histograms.mappings();
    println!("{} quantized tensors", mappings.len());
    report_tensor_loss(model, &f32s, &inputs, &mappings);

    let writer = quantize_file(model, &f32s, &mappings)?;
    let manifest = writer.manifest();
    let bytes = writer.finish();
    fs::write(out, &bytes).with_context(|| format!("writing {}", out.display()))?;
    fs::write(out.with_extension("txt"), &manifest)?;
    println!(
        "wrote {} ({} bytes, sha256 {:x})",
        out.display(),
        bytes.len(),
        Sha256::digest(&bytes)
    );

    let mut i8s = Tensors::read(out)?;
    i8s.pack_for_lanes();
    match model {
        Model::Edgeface => report_edgeface(&f32s, &i8s, &inputs),
        Model::Yunet => report_yunet(&f32s, &i8s, &inputs),
    }
    Ok(())
}

/// One calibration sample: the `f32` network input and the `i8` one.
struct Sample {
    /// The image name.
    name: String,
    /// The `f32` input.
    f32s: Vec<f32>,
    /// The `i8` input.
    i8s: Vec<i8>,
}

/// The samples of `dir` as network inputs.
fn load_samples(model: Model, dir: &Path, limit: Option<usize>) -> Result<Vec<Sample>> {
    let mut paths = data::photos(dir)?;
    paths.truncate(limit.unwrap_or(usize::MAX));
    let mut samples = Vec::new();
    for path in paths {
        let photo = image::open(&path)
            .with_context(|| format!("reading {}", path.display()))?
            .to_rgb8();
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("sample")
            .to_string();
        let (width, height) = (photo.width() as usize, photo.height() as usize);
        let sample = match model {
            Model::Edgeface => {
                if (width, height) != (CROP_SIZE, CROP_SIZE) {
                    println!("skip {name}: not {CROP_SIZE}x{CROP_SIZE}");
                    continue;
                }
                let crop = RgbImage::new(photo.as_raw(), width, height);
                let mut f32s = vec![0.0f32; CROP_SIZE * CROP_SIZE * 3];
                let mut i8s = vec![0i8; CROP_SIZE * CROP_SIZE * 3];
                recognizer_input(&crop, &mut f32s);
                recognizer_input_i8(&crop, &mut i8s);
                Sample { name, f32s, i8s }
            }
            Model::Yunet => {
                if (width, height) != (320, 240) {
                    println!("skip {name}: not 320x240");
                    continue;
                }
                // The board's path: an RGB565 frame, scaled down by 4.
                let frame_bytes = to_rgb565(photo.as_raw());
                let frame = Rgb565Frame::new(&frame_bytes, width, height);
                let mut small = vec![0u8; CONTENT_WIDTH * CONTENT_HEIGHT * 3];
                let mut small_image = RgbImageMut::new(&mut small, CONTENT_WIDTH, CONTENT_HEIGHT);
                downscale_to_rgb(&frame, DOWNSCALE, &mut small_image);
                let mut f32s = vec![0.0f32; yunet::INPUT_SHAPE.len()];
                let mut i8s = vec![0i8; yunet::INPUT_SHAPE.len()];
                detector_input(&small_image.as_image(), &mut f32s);
                detector_input_i8(&small_image.as_image(), &mut i8s);
                Sample { name, f32s, i8s }
            }
        };
        samples.push(sample);
    }
    Ok(samples)
}

/// RGB888 bytes as big-endian RGB565, as the camera delivers.
fn to_rgb565(rgb: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(rgb.len() / 3 * 2);
    for pixel in rgb.chunks_exact(3) {
        let value = (u16::from(pixel[0] >> 3) << 11)
            | (u16::from(pixel[1] >> 2) << 5)
            | u16::from(pixel[2] >> 3);
        frame.extend_from_slice(&value.to_be_bytes());
    }
    frame
}

/// Run the `f32` network with its probes on every sample.
fn run_probed(
    model: Model,
    weights: &Tensors,
    samples: &[Sample],
    mut probe: impl FnMut(&str, Granularity, &[f32]),
) {
    match model {
        Model::Edgeface => {
            let mut scratch = vec![0.0f32; edgeface::SCRATCH_LEN];
            let mut embedding = vec![0.0f32; edgeface::EMBEDDING_LEN];
            for sample in samples {
                edgeface::forward_probed(
                    weights,
                    &sample.f32s,
                    &mut scratch,
                    &mut embedding,
                    |_, _, _| {},
                    &mut probe,
                );
            }
        }
        Model::Yunet => {
            let mut scratch = vec![0.0f32; yunet::SCRATCH_LEN];
            for sample in samples {
                yunet::forward_probed(
                    weights,
                    &sample.f32s,
                    &mut scratch,
                    |_, _, _| {},
                    &mut probe,
                );
            }
        }
    }
}

/// The range of every probed tensor (per channel where asked) over the
/// samples.
fn calibrate(model: Model, weights: &Tensors, samples: &[Sample]) -> Ranges {
    let mut ranges = Ranges::new();
    run_probed(model, weights, samples, |name, granularity, values| {
        let channels = granularity.channels();
        let entry = ranges.entry(name.to_string()).or_insert_with(|| {
            (
                vec![(f32::INFINITY, f32::NEG_INFINITY); channels],
                granularity.wide(),
            )
        });
        for (index, &value) in values.iter().enumerate() {
            let slot = &mut entry.0[index % channels];
            slot.0 = slot.0.min(value);
            slot.1 = slot.1.max(value);
        }
    });
    ranges
}

/// For every probed tensor: the signal-to-noise ratio of quantizing it
/// with its mapping and dequantizing again, worst over the samples, and
/// the clip range. This says which tensors int8 cannot hold.
fn report_tensor_loss(model: Model, weights: &Tensors, samples: &[Sample], mappings: &Mappings) {
    let mut worst: BTreeMap<String, f32> = BTreeMap::new();
    let mut order: Vec<String> = Vec::new();
    run_probed(model, weights, samples, |name, _, values| {
        let (quants, wide) = &mappings[name];
        let back: Vec<f32> = values
            .iter()
            .enumerate()
            .map(|(index, &v)| {
                let quant = quants[index % quants.len()];
                if *wide {
                    quant.dequantize16(quant.quantize16(v))
                } else {
                    quant.dequantize(quant.quantize(v))
                }
            })
            .collect();
        let snr = snr_db(values, &back);
        if !worst.contains_key(name) {
            order.push(name.to_string());
        }
        let slot = worst.entry(name.to_string()).or_insert(f32::INFINITY);
        *slot = slot.min(snr);
    });
    println!("quantization loss per tensor (worst sample), with the mapping's range:");
    for name in &order {
        let (quants, wide) = &mappings[name];
        let snr = worst[name];
        let mark = if snr < 30.0 { "  <--" } else { "" };
        if *wide && quants.len() > 1 {
            println!(
                "  {name:<44} {snr:6.1} dB   16-bit per channel ({}){mark}",
                quants.len()
            );
        } else if *wide {
            let q = quants[0];
            println!(
                "  {name:<44} {snr:6.1} dB   16-bit, +-{:.3}{mark}",
                q.scale * 32767.0
            );
        } else if quants.len() == 1 {
            let q = quants[0];
            let (low, high) = (q.dequantize(-128), q.dequantize(127));
            println!("  {name:<44} {snr:6.1} dB   [{low:8.3}, {high:8.3}]{mark}");
        } else {
            println!(
                "  {name:<44} {snr:6.1} dB   per channel ({}){mark}",
                quants.len()
            );
        }
    }
}

/// The LayerNorms of EdgeFace whose scale and shift fold into the layer
/// after them: `(norm prefix, consumer prefix)`. The stem's LayerNorm
/// feeds the `f32` stream and stays.
fn folded_norms(f32s: &Tensors) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    for name in &f32s.names {
        // The head first: its name also ends in `.norm.weight`.
        if name == "head.norm.weight" {
            pairs.push(("head.norm".to_string(), "head.fc".to_string()));
        } else if let Some(prefix) = name.strip_suffix(".norm.weight") {
            pairs.push((format!("{prefix}.norm"), format!("{prefix}.mlp.fc1")));
        } else if let Some(prefix) = name.strip_suffix(".norm_xca.weight") {
            pairs.push((format!("{prefix}.norm_xca"), format!("{prefix}.xca.qkv")));
        } else if let Some(prefix) = name.strip_suffix(".downsample.0.weight") {
            pairs.push((
                format!("{prefix}.downsample.0"),
                format!("{prefix}.downsample.1"),
            ));
        }
    }
    // Every consumer must exist: a typo here would silently skip a fold.
    for (norm, consumer) in &pairs {
        assert!(
            f32s.by_name.contains_key(&format!("{consumer}.weight")),
            "{norm} folds into missing {consumer}"
        );
    }
    pairs
}

/// `weight` and `bias` of the layer after a LayerNorm with `gamma` and
/// `beta`, with the LayerNorm's affine part folded in:
/// `W(gamma * x + beta) + b = (W diag gamma) x + (W beta + b)`. The
/// input channel is the innermost dimension of the weight (`OI`, `OHWI`).
fn fold_affine(weight: &[f32], bias: &[f32], gamma: &[f32], beta: &[f32]) -> (Vec<f32>, Vec<f32>) {
    let channels = gamma.len();
    let out_channels = bias.len();
    let per_output = weight.len() / out_channels;
    let mut folded = vec![0.0f32; weight.len()];
    let mut folded_bias = bias.to_vec();
    for o in 0..out_channels {
        for (index, &w) in weight[o * per_output..(o + 1) * per_output]
            .iter()
            .enumerate()
        {
            let i = index % channels;
            folded[o * per_output + index] = w * gamma[i];
            folded_bias[o] += w * beta[i];
        }
    }
    (folded, folded_bias)
}

/// The integer file: every weight quantized per output channel (with the
/// LayerNorm affines folded in first), every other tensor copied, and the
/// `q.<name>` mappings.
fn quantize_file(model: Model, f32s: &Tensors, mappings: &Mappings) -> Result<Writer> {
    let folds: BTreeMap<String, String> = match model {
        Model::Edgeface => folded_norms(f32s)
            .into_iter()
            .map(|(norm, consumer)| (consumer, norm))
            .collect(),
        Model::Yunet => BTreeMap::new(),
    };
    // The biases of folded layers are written with their weights; the
    // originals are skipped when the file order reaches them.
    let folded_biases: std::collections::BTreeSet<String> =
        folds.keys().map(|base| format!("{base}.bias")).collect();
    let mut writer = Writer::new();
    for name in &f32s.names {
        let tensor = f32s.tensor(name);
        if folded_biases.contains(name) {
            continue;
        }
        let Some(base) = name.strip_suffix(".weight") else {
            writer.add_f32(name, &tensor.layout, &tensor.shape, &tensor.f32s)?;
            continue;
        };
        let (values, bias) = match folds.get(base) {
            Some(norm) => {
                let bias = &f32s.tensor(&format!("{base}.bias")).f32s;
                let gamma = &f32s.tensor(&format!("{norm}.weight")).f32s;
                let beta = &f32s.tensor(&format!("{norm}.bias")).f32s;
                let (weight, bias) = fold_affine(&tensor.f32s, bias, gamma, beta);
                (weight, Some(bias))
            }
            None => (tensor.f32s.clone(), None),
        };
        let mut data = vec![0i8; values.len()];
        let (scales, layout) = match tensor.layout.as_str() {
            "OHWI" | "OI" => {
                let channel_len: usize = tensor.shape[1..].iter().product();
                let mut scales = vec![0.0f32; tensor.shape[0]];
                quantize_weight_rows(&values, channel_len, &mut data, &mut scales);
                (scales, "O")
            }
            "HWC" => {
                let channels = tensor.shape[2];
                let mut scales = vec![0.0f32; channels];
                quantize_weight_channels(&values, channels, &mut data, &mut scales);
                (scales, "C")
            }
            // LayerNorm weights and the like: not a matrix, stays f32.
            _ => {
                writer.add_f32(name, &tensor.layout, &tensor.shape, &tensor.f32s)?;
                continue;
            }
        };
        writer.add(Tensor {
            name: name.clone(),
            layout: tensor.layout.clone(),
            data_type: DataType::I8,
            dims: tensor.shape.clone(),
            scale: 0.0,
            zero_point: 0,
            data: data.iter().map(|&q| q as u8).collect(),
        })?;
        writer.add_f32(&format!("{base}.scales"), layout, &[scales.len()], &scales)?;
        if let Some(bias) = bias {
            writer.add_f32(&format!("{base}.bias"), "O", &[bias.len()], &bias)?;
        }
    }
    for (name, (quants, wide)) in mappings {
        let pairs: Vec<f32> = quants
            .iter()
            .flat_map(|q| [q.scale, q.zero_point as f32])
            .collect();
        if *wide && quants.len() > 1 {
            writer.add_f32(&format!("q.{name}"), "QC16", &[quants.len(), 2], &pairs)?;
        } else if *wide {
            writer.add_f32(&format!("q.{name}"), "Q16", &[2], &pairs)?;
        } else if quants.len() == 1 {
            writer.add_f32(&format!("q.{name}"), "Q", &[2], &pairs)?;
        } else {
            writer.add_f32(&format!("q.{name}"), "QC", &[quants.len(), 2], &pairs)?;
        }
    }
    Ok(writer)
}

/// Per-block signal-to-noise ratios, kept as the minimum over samples.
struct SnrTable {
    /// Block name to its worst ratio and the largest values seen.
    rows: BTreeMap<String, (usize, f32)>,
}

impl SnrTable {
    /// An empty table.
    fn new() -> Self {
        Self {
            rows: BTreeMap::new(),
        }
    }

    /// Record `snr` for `name`, keeping the file order by `index`.
    fn record(&mut self, index: usize, name: &str, snr: f32) {
        let row = self
            .rows
            .entry(name.to_string())
            .or_insert((index, f32::INFINITY));
        row.1 = row.1.min(snr);
    }

    /// Print the table in graph order and return the worst ratio.
    fn print(&self) -> f32 {
        let mut rows: Vec<_> = self.rows.iter().collect();
        rows.sort_by_key(|(_, (index, _))| *index);
        let mut worst = f32::INFINITY;
        for (name, (_, snr)) in rows {
            let mark = if *snr < 30.0 { "  <-- below 30 dB" } else { "" };
            println!("{name:<44} {snr:6.1} dB{mark}");
            worst = worst.min(*snr);
        }
        worst
    }
}

/// Compare the integer recognizer with the `f32` one on every sample.
fn report_edgeface(f32s: &Tensors, i8s: &Tensors, samples: &[Sample]) {
    let mut scratch = vec![0.0f32; edgeface::SCRATCH_LEN];
    let mut runner = Runner::new();
    let mut reference = vec![0.0f32; edgeface::EMBEDDING_LEN];
    let mut embedding = vec![0.0f32; edgeface::EMBEDDING_LEN];
    let mut table = SnrTable::new();
    let (mut worst_cosine, mut sum_cosine) = (1.0f32, 0.0f32);
    for sample in samples {
        let mut traced: Vec<(String, Vec<f32>)> = Vec::new();
        edgeface::forward_traced(
            f32s,
            &sample.f32s,
            &mut scratch,
            &mut reference,
            |name, _, values| {
                traced.push((name.to_string(), values.to_vec()));
            },
        );
        // The integer pass traces the same blocks under the same names,
        // except the LayerNorm outputs it folds away; match by name.
        runner.forward_traced(i8s, &sample.i8s, &mut embedding, |name, _, values| {
            if let Some(index) = traced.iter().position(|(expected, _)| expected == name) {
                table.record(index, name, snr_db(&traced[index].1, values));
            }
        });
        let cosine = cosine(&reference, &embedding);
        worst_cosine = worst_cosine.min(cosine);
        sum_cosine += cosine;
        if cosine < 0.99 {
            println!("{}: cosine {cosine:.4}", sample.name);
        }
    }
    let worst = table.print();
    println!(
        "embedding cosine int8 vs f32: worst {worst_cosine:.4}, mean {:.4} over {} samples; worst block SNR {worst:.1} dB",
        sum_cosine / samples.len() as f32,
        samples.len()
    );
}

/// Compare the integer detector with the `f32` one on every sample.
fn report_yunet(f32s: &Tensors, i8s: &Tensors, samples: &[Sample]) {
    let mut scratch = vec![0.0f32; yunet::SCRATCH_LEN];
    let mut scratch_i16 = vec![0i16; yunet::int8::SCRATCH_I16_LEN + 8];
    let mut scratch_f32 = vec![0.0f32; yunet::int8::F32_SCRATCH_LEN];
    let mut padded = vec![0i8; yunet::int8::MODEL_WEIGHTS_LEN + 16];
    let mut plans = vec![GroupPlan::ZERO; yunet::int8::MODEL_PLANS];
    let model = yunet::int8::Model::compile(
        i8s,
        yunet::int8::ModelStorage {
            weights: crate::recognizer::aligned(&mut padded),
            plans: &mut plans,
        },
    );
    let mut table = SnrTable::new();
    let (mut worst_centre, mut worst_size, mut worst_landmark, mut worst_score) =
        (0.0f32, 0.0f32, 0.0f32, 0.0f32);
    let (mut agree, mut compared) = (0usize, 0usize);
    for sample in samples {
        let mut traced: Vec<(String, Vec<f32>)> = Vec::new();
        let heads = yunet::forward_traced(f32s, &sample.f32s, &mut scratch, |name, _, values| {
            traced.push((name.to_string(), values.to_vec()));
        });
        let reference = decode(&heads, DEFAULT_SCORE_THRESHOLD, DEFAULT_NMS_THRESHOLD);
        let heads = model.forward_traced(
            &sample.i8s,
            yunet::int8::Scratch::new(
                crate::recognizer::aligned(&mut scratch_i16),
                &mut scratch_f32,
            ),
            |name, _, values| {
                if let Some(index) = traced.iter().position(|(expected, _)| expected == name) {
                    table.record(index, name, snr_db(&traced[index].1, values));
                }
            },
        );
        let faces = decode(&heads, DEFAULT_SCORE_THRESHOLD, DEFAULT_NMS_THRESHOLD);
        compared += 1;
        match (reference.best(), faces.best()) {
            (Some(a), Some(b)) => {
                agree += 1;
                let centre = ((a.centre()[0] - b.centre()[0]).powi(2)
                    + (a.centre()[1] - b.centre()[1]).powi(2))
                .sqrt();
                let size = (a.width - b.width).abs().max((a.height - b.height).abs());
                let landmark = a
                    .landmarks
                    .iter()
                    .zip(&b.landmarks)
                    .map(|(p, q)| ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2)).sqrt())
                    .fold(0.0f32, f32::max);
                worst_centre = worst_centre.max(centre);
                worst_size = worst_size.max(size);
                worst_landmark = worst_landmark.max(landmark);
                worst_score = worst_score.max((a.score - b.score).abs());
                if centre > 2.0 || size > 2.0 || landmark > 2.0 {
                    println!(
                        "{}: centre {centre:.2} px, size {size:.2} px, landmark {landmark:.2} px, score {:.3} vs {:.3}",
                        sample.name, a.score, b.score
                    );
                }
            }
            (None, None) => agree += 1,
            (a, b) => println!(
                "{}: f32 {} face(s), int8 {} face(s)",
                sample.name,
                a.is_some() as u8,
                b.is_some() as u8
            ),
        }
    }
    let worst = table.print();
    println!(
        "detections agree on {agree} of {compared} frames; worst centre {worst_centre:.2} px, size {worst_size:.2} px, landmark {worst_landmark:.2} px, score {worst_score:.3} (detector pixels); worst node SNR {worst:.1} dB"
    );
    let _ = Shape::new(0, 0, 0);
}

/// Cosine similarity.
fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norm = |v: &[f32]| v.iter().map(|x| x * x).sum::<f32>().sqrt();
    dot / (norm(a) * norm(b))
}
