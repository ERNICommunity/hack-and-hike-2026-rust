//! Neural-network layers in `f32`, and the two networks built from them.
//!
//! A network is a fixed sequence of arithmetic over arrays of numbers.
//! This module has one function for each kind of arithmetic the two models
//! use (a "kernel"), and a module for each model that calls the kernels in
//! the model's order with the model's weights:
//!
//! - [`yunet`]: the face detector, convolutions only,
//! - [`edgeface`]: the face recognizer, convolutions, layer normalization,
//!   small matrix products and attention,
//! - [`quant`]: the integer versions of the expensive kernels, and how
//!   tensors move between `f32` and `i8`.
//!
//! Everything here is the `f32` reference: plain loops, written for
//! clarity and checked against the models' original outputs (see the
//! golden files in `tests/fixtures/`). The faster integer kernels of a
//! later step are checked against these.
//!
//! # Layout
//!
//! An activation tensor is a slice of `f32` with a [`Shape`]: rows, then
//! columns, then channels, channels innermost ("channels-last"). Pixel
//! `(x, y)` starts at index `(y * width + x) * channels`. The weights come
//! from an FKB1 file, already arranged for this layout (see
//! `tools/facekit/src/export.rs` for the table).
//!
//! # Buffers
//!
//! Kernels never allocate. The caller passes the output slice, which must
//! have room for the whole result; the kernel returns the output's shape.
//! The task stacks on the board are small, so callers keep these buffers
//! in the heap or in PSRAM, never on the stack.

pub mod edgeface;
pub mod lanes;
pub mod pack;
pub mod quant;
pub mod yunet;

use crate::blob::{Blob, BlobError};

/// The dimensions of a channels-last activation tensor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shape {
    /// Rows.
    pub height: usize,
    /// Columns.
    pub width: usize,
    /// Values per pixel.
    pub channels: usize,
}

impl Shape {
    /// A shape.
    pub const fn new(height: usize, width: usize, channels: usize) -> Self {
        Self {
            height,
            width,
            channels,
        }
    }

    /// The number of values in a tensor of this shape.
    pub const fn len(&self) -> usize {
        self.height * self.width * self.channels
    }

    /// Whether the tensor has no values.
    pub const fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The number of pixels (tokens, for the attention layers).
    pub const fn pixels(&self) -> usize {
        self.height * self.width
    }

    /// The index of the first channel of pixel `(x, y)`.
    pub const fn offset(&self, x: usize, y: usize) -> usize {
        (y * self.width + x) * self.channels
    }
}

/// Where a network gets its weights: by name, as `f32` slices. The tests
/// implement it over an FKB1 file read into memory; the firmware
/// implements it over the file in flash.
pub trait Weights {
    /// The values of the `f32` tensor called `name`, in the layout facekit
    /// exported them in.
    ///
    /// # Panics
    ///
    /// Implementations panic when the tensor does not exist: a missing
    /// weight is a programming error, not a run-time condition.
    fn get(&self, name: &str) -> &[f32];

    /// The values of the `i8` tensor called `name`, for the integer
    /// networks. Same rules as [`Weights::get`].
    fn get_i8(&self, name: &str) -> &[i8];

    /// Whether the `i8` tensor called `name` is grouped by eight output
    /// channels for the vector unit (see [`pack`]). Plain files say no.
    fn packed(&self, _name: &str) -> bool {
        false
    }
}

/// [`Weights`] over an FKB1 file in flash.
///
/// Reading a tensor is a lookup by name in the file's table of contents
/// and a borrow of its bytes: no copying, no allocation, and the data
/// stays in flash. Build it once and keep it for as long as the
/// application runs.
///
/// A missing or misaligned tensor panics, because both mean the file and
/// the code disagree, which no application can recover from. Use
/// [`include_fkb!`](crate::include_fkb) so the alignment is right.
pub struct BlobWeights<'a> {
    /// The parsed file.
    blob: Blob<'a>,
}

impl<'a> BlobWeights<'a> {
    /// Parse `bytes`.
    ///
    /// # Errors
    ///
    /// When the bytes are not a valid FKB1 file.
    pub fn new(bytes: &'a [u8]) -> Result<Self, BlobError> {
        Ok(Self {
            blob: Blob::parse(bytes)?,
        })
    }

    /// The file, for callers that need more than the weights.
    pub fn blob(&self) -> &Blob<'a> {
        &self.blob
    }
}

impl Weights for BlobWeights<'_> {
    fn get(&self, name: &str) -> &[f32] {
        let entry = self
            .blob
            .get(name)
            .unwrap_or_else(|| panic!("the weights file has no tensor {name}"));
        entry
            .f32_slice()
            .unwrap_or_else(|| panic!("tensor {name} is not aligned for f32; use include_fkb!"))
    }

    fn get_i8(&self, name: &str) -> &[i8] {
        self.blob
            .get(name)
            .unwrap_or_else(|| panic!("the weights file has no tensor {name}"))
            .i8_slice()
    }

    fn packed(&self, name: &str) -> bool {
        self.blob
            .get(name)
            .unwrap_or_else(|| panic!("the weights file has no tensor {name}"))
            .layout
            .ends_with(pack::PACKED_SUFFIX)
    }
}

/// What happens to each output value of a convolution.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Activation {
    /// Nothing.
    None,
    /// `max(0, x)`.
    Relu,
}

impl Activation {
    /// Apply to one value.
    fn apply(self, value: f32) -> f32 {
        match self {
            Self::None => value,
            Self::Relu => value.max(0.0),
        }
    }
}

/// The size of one dimension after a convolution or pooling with the given
/// kernel size, symmetric padding and stride: `(size + 2p - k) / s + 1`,
/// rounded down, as ONNX defines it.
pub(super) fn output_size(size: usize, kernel: usize, padding: usize, stride: usize) -> usize {
    (size + 2 * padding - kernel) / stride + 1
}

/// A full convolution: every output channel sees every input channel.
///
/// `weight` is `[out channels][kernel rows][kernel columns][in channels]`
/// (layout `OHWI`), `bias` has one value per output channel. The kernel is
/// square, the padding is symmetric with zeros outside the image. Returns
/// the output shape.
///
/// # Panics
///
/// When the slices do not match the shapes.
// A convolution has exactly these parameters; a struct would only move
// them. The integer kernels of a later step take the same list.
#[allow(clippy::too_many_arguments)]
pub fn conv2d(
    input: &[f32],
    shape: Shape,
    weight: &[f32],
    bias: &[f32],
    kernel: usize,
    stride: usize,
    padding: usize,
    activation: Activation,
    output: &mut [f32],
) -> Shape {
    let out_channels = bias.len();
    let out = Shape::new(
        output_size(shape.height, kernel, padding, stride),
        output_size(shape.width, kernel, padding, stride),
        out_channels,
    );
    assert_eq!(input.len(), shape.len(), "conv2d input");
    assert_eq!(
        weight.len(),
        out_channels * kernel * kernel * shape.channels,
        "conv2d weight"
    );
    assert!(output.len() >= out.len(), "conv2d output buffer");

    let taps = kernel * kernel * shape.channels;
    for oy in 0..out.height {
        for ox in 0..out.width {
            let pixel = &mut output[out.offset(ox, oy)..out.offset(ox, oy) + out_channels];
            for (o, slot) in pixel.iter_mut().enumerate() {
                let filter = &weight[o * taps..(o + 1) * taps];
                let mut sum = bias[o];
                for ky in 0..kernel {
                    let Some(iy) = (oy * stride + ky).checked_sub(padding) else {
                        continue;
                    };
                    if iy >= shape.height {
                        continue;
                    }
                    for kx in 0..kernel {
                        let Some(ix) = (ox * stride + kx).checked_sub(padding) else {
                            continue;
                        };
                        if ix >= shape.width {
                            continue;
                        }
                        let source =
                            &input[shape.offset(ix, iy)..shape.offset(ix, iy) + shape.channels];
                        let tap = (ky * kernel + kx) * shape.channels;
                        for (value, w) in source.iter().zip(&filter[tap..tap + shape.channels]) {
                            sum += value * w;
                        }
                    }
                }
                *slot = activation.apply(sum);
            }
        }
    }
    out
}

/// A depthwise convolution: each channel is filtered on its own.
///
/// `weight` is `[kernel rows][kernel columns][channels]` (layout `HWC`),
/// `bias` has one value per channel. Square kernel, symmetric zero padding.
/// Returns the output shape, which has the input's channel count.
///
/// # Panics
///
/// When the slices do not match the shapes.
// A convolution has exactly these parameters; a struct would only move
// them. The integer kernels of a later step take the same list.
#[allow(clippy::too_many_arguments)]
pub fn depthwise(
    input: &[f32],
    shape: Shape,
    weight: &[f32],
    bias: &[f32],
    kernel: usize,
    stride: usize,
    padding: usize,
    activation: Activation,
    output: &mut [f32],
) -> Shape {
    let channels = shape.channels;
    let out = Shape::new(
        output_size(shape.height, kernel, padding, stride),
        output_size(shape.width, kernel, padding, stride),
        channels,
    );
    assert_eq!(input.len(), shape.len(), "depthwise input");
    assert_eq!(weight.len(), kernel * kernel * channels, "depthwise weight");
    assert_eq!(bias.len(), channels, "depthwise bias");
    assert!(output.len() >= out.len(), "depthwise output buffer");

    for oy in 0..out.height {
        for ox in 0..out.width {
            let pixel = &mut output[out.offset(ox, oy)..out.offset(ox, oy) + channels];
            pixel.copy_from_slice(bias);
            for ky in 0..kernel {
                let Some(iy) = (oy * stride + ky).checked_sub(padding) else {
                    continue;
                };
                if iy >= shape.height {
                    continue;
                }
                for kx in 0..kernel {
                    let Some(ix) = (ox * stride + kx).checked_sub(padding) else {
                        continue;
                    };
                    if ix >= shape.width {
                        continue;
                    }
                    let source = &input[shape.offset(ix, iy)..shape.offset(ix, iy) + channels];
                    let tap = (ky * kernel + kx) * channels;
                    for ((slot, value), w) in pixel
                        .iter_mut()
                        .zip(source)
                        .zip(&weight[tap..tap + channels])
                    {
                        *slot += value * w;
                    }
                }
            }
            for slot in pixel.iter_mut() {
                *slot = activation.apply(*slot);
            }
        }
    }
    out
}

/// A linear layer on every row of `input`: `out = W * in + b`, where each
/// row has `in_features` values. A 1x1 convolution is the same thing with
/// one row per pixel, and so is a token-wise MLP layer.
///
/// `weight` is `[out features][in features]` (layout `OI`); `bias` is
/// optional. Returns the number of rows.
///
/// # Panics
///
/// When the slices do not match.
pub fn linear(
    input: &[f32],
    in_features: usize,
    weight: &[f32],
    bias: Option<&[f32]>,
    out_features: usize,
    activation: Activation,
    output: &mut [f32],
) -> usize {
    assert!(
        in_features > 0 && input.len().is_multiple_of(in_features),
        "linear input"
    );
    assert_eq!(weight.len(), out_features * in_features, "linear weight");
    let rows = input.len() / in_features;
    assert!(output.len() >= rows * out_features, "linear output buffer");
    for (source, target) in input
        .chunks_exact(in_features)
        .zip(output.chunks_exact_mut(out_features))
    {
        for (o, slot) in target.iter_mut().enumerate() {
            let filter = &weight[o * in_features..(o + 1) * in_features];
            let mut sum = bias.map_or(0.0, |bias| bias[o]);
            for (value, w) in source.iter().zip(filter) {
                sum += value * w;
            }
            *slot = activation.apply(sum);
        }
    }
    rows
}

/// Layer normalization over each row of `channels` values, in place:
/// subtract the row's mean, divide by the square root of its variance plus
/// `epsilon`, then scale and shift per channel with `weight` and `bias`.
/// The same as [`standardize_rows`] followed by [`affine_rows`].
///
/// # Panics
///
/// When the slices do not match.
pub fn layer_norm(data: &mut [f32], channels: usize, weight: &[f32], bias: &[f32], epsilon: f32) {
    standardize_rows(data, channels, epsilon);
    affine_rows(data, weight, bias);
}

/// The first half of layer normalization: each row of `channels` values
/// gets mean 0 and variance 1 (plus `epsilon`), in place.
///
/// # Panics
///
/// When the length is not a multiple of `channels`.
pub fn standardize_rows(data: &mut [f32], channels: usize, epsilon: f32) {
    assert!(
        channels > 0 && data.len().is_multiple_of(channels),
        "standardize_rows data"
    );
    let count = channels as f32;
    for row in data.chunks_exact_mut(channels) {
        let mean = row.iter().sum::<f32>() / count;
        let variance = row
            .iter()
            .map(|value| (value - mean) * (value - mean))
            .sum::<f32>()
            / count;
        let scale = 1.0 / libm::sqrtf(variance + epsilon);
        for value in row.iter_mut() {
            *value = (*value - mean) * scale;
        }
    }
}

/// The second half of layer normalization: `value * weight[c] + bias[c]`
/// for channel `c` of every row, in place.
///
/// # Panics
///
/// When the slices do not match.
pub fn affine_rows(data: &mut [f32], weight: &[f32], bias: &[f32]) {
    let channels = weight.len();
    assert!(
        channels > 0 && data.len().is_multiple_of(channels),
        "affine_rows data"
    );
    assert_eq!(bias.len(), channels, "affine_rows bias");
    for row in data.chunks_exact_mut(channels) {
        for ((value, w), b) in row.iter_mut().zip(weight).zip(bias) {
            *value = *value * w + b;
        }
    }
}

/// GELU (Gaussian error linear unit), the exact form
/// `x * 0.5 * (1 + erf(x / sqrt 2))`, in place.
pub fn gelu(data: &mut [f32]) {
    for value in data.iter_mut() {
        *value = *value * 0.5 * (1.0 + libm::erff(*value * core::f32::consts::FRAC_1_SQRT_2));
    }
}

/// The logistic function `1 / (1 + exp(-x))`, in place.
pub fn sigmoid(data: &mut [f32]) {
    for value in data.iter_mut() {
        *value = 1.0 / (1.0 + libm::expf(-*value));
    }
}

/// Softmax over each row of `row_len` values, in place: `exp(x - max)`,
/// divided by the row's sum.
///
/// # Panics
///
/// When the length is not a multiple of `row_len`.
pub fn softmax_rows(data: &mut [f32], row_len: usize) {
    assert!(
        row_len > 0 && data.len().is_multiple_of(row_len),
        "softmax_rows data"
    );
    for row in data.chunks_exact_mut(row_len) {
        let max = row.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let mut sum = 0.0;
        for value in row.iter_mut() {
            *value = libm::expf(*value - max);
            sum += *value;
        }
        for value in row.iter_mut() {
            *value /= sum;
        }
    }
}

/// Divide each row of `row_len` values by its length, in place, like
/// PyTorch's `F.normalize`: the length is clamped to at least `epsilon`.
///
/// # Panics
///
/// When the length is not a multiple of `row_len`.
pub fn l2_normalize_rows(data: &mut [f32], row_len: usize, epsilon: f32) {
    assert!(
        row_len > 0 && data.len().is_multiple_of(row_len),
        "l2_normalize_rows data"
    );
    for row in data.chunks_exact_mut(row_len) {
        let norm = libm::sqrtf(row.iter().map(|value| value * value).sum::<f32>()).max(epsilon);
        for value in row.iter_mut() {
            *value /= norm;
        }
    }
}

/// 2x2 max pooling with stride 2. An odd last row or column is dropped,
/// as ONNX does. Returns the output shape.
///
/// # Panics
///
/// When the slices do not match the shapes.
pub fn max_pool_2x2(input: &[f32], shape: Shape, output: &mut [f32]) -> Shape {
    let out = Shape::new(shape.height / 2, shape.width / 2, shape.channels);
    assert_eq!(input.len(), shape.len(), "max_pool_2x2 input");
    assert!(output.len() >= out.len(), "max_pool_2x2 output buffer");
    for oy in 0..out.height {
        for ox in 0..out.width {
            let target = &mut output[out.offset(ox, oy)..out.offset(ox, oy) + shape.channels];
            for (c, slot) in target.iter_mut().enumerate() {
                let mut max = f32::NEG_INFINITY;
                for (dy, dx) in [(0, 0), (0, 1), (1, 0), (1, 1)] {
                    max = max.max(input[shape.offset(ox * 2 + dx, oy * 2 + dy) + c]);
                }
                *slot = max;
            }
        }
    }
    out
}

/// Add `small`, enlarged 2x by repeating each pixel, to `target`, in place.
/// `target` has twice the rows and columns of `small` (the neck of YuNet
/// enlarges a coarse map and adds it to the finer one).
///
/// # Panics
///
/// When the shapes do not match.
pub fn upsample_2x_add(small: &[f32], small_shape: Shape, target: &mut [f32], target_shape: Shape) {
    assert_eq!(small.len(), small_shape.len(), "upsample_2x_add small");
    assert_eq!(target.len(), target_shape.len(), "upsample_2x_add target");
    assert_eq!(
        (
            target_shape.height,
            target_shape.width,
            target_shape.channels
        ),
        (
            small_shape.height * 2,
            small_shape.width * 2,
            small_shape.channels
        ),
        "upsample_2x_add shapes"
    );
    let channels = small_shape.channels;
    for y in 0..target_shape.height {
        for x in 0..target_shape.width {
            let source = &small
                [small_shape.offset(x / 2, y / 2)..small_shape.offset(x / 2, y / 2) + channels];
            let slot = &mut target[target_shape.offset(x, y)..target_shape.offset(x, y) + channels];
            for (t, s) in slot.iter_mut().zip(source) {
                *t += s;
            }
        }
    }
}

/// The mean over all pixels of each channel: `shape.channels` values.
///
/// # Panics
///
/// When the slices do not match the shape.
pub fn global_average(input: &[f32], shape: Shape, output: &mut [f32]) {
    assert_eq!(input.len(), shape.len(), "global_average input");
    assert!(
        output.len() >= shape.channels,
        "global_average output buffer"
    );
    let output = &mut output[..shape.channels];
    output.fill(0.0);
    for pixel in input.chunks_exact(shape.channels) {
        for (sum, value) in output.iter_mut().zip(pixel) {
            *sum += value;
        }
    }
    let count = shape.pixels() as f32;
    for sum in output.iter_mut() {
        *sum /= count;
    }
}

/// `target[i] += source[i] * scale[i % channels]`: add a per-channel
/// scaled tensor to another, in place. `scale` is a layer-scale vector
/// (`gamma`); pass all ones for a plain residual add.
///
/// # Panics
///
/// When the lengths do not match.
pub fn add_scaled(target: &mut [f32], source: &[f32], scale: &[f32]) {
    assert_eq!(target.len(), source.len(), "add_scaled lengths");
    let channels = scale.len();
    assert!(
        channels > 0 && target.len().is_multiple_of(channels),
        "add_scaled channels"
    );
    for (t, s) in target
        .chunks_exact_mut(channels)
        .zip(source.chunks_exact(channels))
    {
        for ((t, s), g) in t.iter_mut().zip(s).zip(scale) {
            *t += s * g;
        }
    }
}

/// `target[i] += source[i]`, in place.
///
/// # Panics
///
/// When the lengths do not match.
pub fn add(target: &mut [f32], source: &[f32]) {
    assert_eq!(target.len(), source.len(), "add lengths");
    for (t, s) in target.iter_mut().zip(source) {
        *t += s;
    }
}
