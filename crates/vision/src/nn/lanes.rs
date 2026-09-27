//! The integer pipeline of the vector unit: kernels whose every value
//! stays in the eight 16-bit lanes from the input to the output.
//!
//! The kernels of [`quant`](super::quant) accumulate exactly and then hand
//! each sum to Rust, which scales it and stores it. On this core that
//! costs sixty to a hundred cycles per output, more than the products
//! themselves. Here a layer's whole output pixel or group of eight
//! channels goes from the accumulator to memory without leaving the
//! vector registers, in the way Espressif's own library does it:
//!
//! 1. The eight 40-bit accumulator lanes start from a *bias image*: the
//!    bias of each channel in units of one product, plus half a unit of
//!    the shift that follows, so the shift rounds instead of truncating.
//! 2. The products are accumulated (16-bit inputs, 8-bit weights
//!    widened in the registers, or 16-bit weights for the attention).
//! 3. `ee.srcmb.s16.qacc` shifts the eight sums right by `s1` (one shift
//!    per group, chosen so the largest lane fits) and saturates them to
//!    16 bits.
//! 4. Each lane is multiplied by its own 16-bit factor `m` (the channel's
//!    scale relative to the output's step) through the accumulator, and
//!    shifted by `s2` with saturation again. A per-lane `half` is added
//!    before the multiply so that this shift rounds too.
//! 5. The result is stored, added to the residual stream, or clamped for
//!    a ReLU; all saturating 16-bit vector operations.
//!
//! Every tensor between layers is `i16` with a symmetric mapping (zero
//! point 0): one scale per tensor, or one per channel for the residual
//! stream, where a few channels are ten times larger than the rest. The
//! computer runs the same arithmetic in scalar code ([`model`]), bit for
//! bit, so `facekit` can measure what the board will compute.
//!
//! What stays scalar: the GELU lookup (a table indexed by the 16-bit
//! value), the softmax of the attention (a few hundred values), and the
//! per-pixel inverse square root of the LayerNorm.

use super::{Shape, output_size, pack};

pub mod model;

/// Channels in a group: the lanes of the accumulator.
pub const LANES: usize = 8;

/// The plan of one group of eight output channels: what the epilogue
/// needs, laid out as the assembly reads it (128 bytes, 16-byte
/// aligned).
#[derive(Clone, Copy, Debug)]
#[repr(C, align(16))]
pub struct GroupPlan {
    /// The accumulator image the group starts from: each lane's bias in
    /// whole product units plus half of `2^s1`, as `ee.ld.qacc` reads it.
    pub image: [u32; 16],
    /// Added to each lane after the first shift: half of `2^s2` in units
    /// of the lane's factor, so the second shift rounds.
    pub half: [i16; LANES],
    /// The factor of each lane: `2^s1 * scale / step * 2^s2`.
    pub factor: [i16; LANES],
    /// Added to each lane after the second shift, in output units: the
    /// part of the bias below one product unit.
    pub offset: [i16; LANES],
    /// The first shift: sums to 16 bits.
    pub s1: u32,
    /// The second shift: factors to output units.
    pub s2: u32,
    /// Padding to 128 bytes.
    pub reserved: [u32; 2],
}

impl GroupPlan {
    /// A plan that produces zeros.
    pub const ZERO: Self = Self {
        image: [0; 16],
        half: [0; LANES],
        factor: [0; LANES],
        offset: [0; LANES],
        s1: 0,
        s2: 0,
        reserved: [0; 2],
    };

    /// The plan for eight channels whose sums, times `scale[j]` (the
    /// product of the input's and the channel's weight scale), plus
    /// `bias[j]`, are real values to be written in units of `step`.
    /// `bound` is the largest product count times the largest input and
    /// weight, or any bound on the magnitude of a sum: it sets the first
    /// shift.
    ///
    /// # Panics
    ///
    /// When a scale is zero.
    pub fn new(scale: &[f32; LANES], bias: &[f32; LANES], step: f32, bound: f32) -> Self {
        Self::with_gain(scale, bias, &[1.0; LANES], 32767.0 * step, step, bound)
    }

    /// The plan for eight channels whose sums, times `scale[j]`, plus
    /// `bias[j]`, are real values whose magnitude stays below `range`,
    /// and which are then multiplied by `gain[j]` (a block's layer scale)
    /// and written in units of `step`. The first shift comes from
    /// `range`, before the gain: a lane with a tiny gain keeps the sum's
    /// precision instead of forcing a wide shift on the whole group.
    ///
    /// # Panics
    ///
    /// When a scale is zero.
    pub fn with_gain(
        scale: &[f32; LANES],
        bias: &[f32; LANES],
        gain: &[f32; LANES],
        range: f32,
        step: f32,
        bound: f32,
    ) -> Self {
        // The first shift: the largest lane must fit 16 bits after it.
        // A lane's magnitude is bounded by the range of the real values
        // plus its bias, divided by its scale, and by the sum itself.
        // The bias in whole product units goes into the image; what is
        // left (below one product unit) is added in output units at the
        // end.
        let mut bias_units = [0.0f32; LANES];
        let mut largest = 0.0f32;
        for j in 0..LANES {
            assert!(scale[j] != 0.0, "lane scale");
            bias_units[j] = libm::roundf(bias[j] / scale[j]);
            let by_output = (range + bias[j].abs()) / scale[j].abs();
            let by_sum = bound + bias_units[j].abs();
            largest = largest.max(by_output.min(by_sum));
        }
        let s1 = if largest > 32767.0 {
            ceil_log2(largest / 32767.0).max(0) as u32
        } else {
            0
        };
        // Each lane's factor: from `2^s1` product units to output steps.
        let mut ratios = [0.0f32; LANES];
        let mut largest_ratio = 0.0f32;
        for j in 0..LANES {
            ratios[j] = scale[j] * gain[j] * pow2(s1 as i32) / step;
            largest_ratio = largest_ratio.max(ratios[j].abs());
        }
        // The second shift: the largest factor just under 2^15.
        let s2 = if largest_ratio > 0.0 {
            (14 - ceil_log2(largest_ratio)).clamp(0, 30) as u32
        } else {
            0
        };
        let mut factor = [0i16; LANES];
        let mut half = [0i16; LANES];
        let mut offset = [0i16; LANES];
        let mut sums = [0i64; LANES];
        for j in 0..LANES {
            let f = libm::roundf(ratios[j] * pow2(s2 as i32));
            factor[j] = f.clamp(-32768.0, 32767.0) as i16;
            half[j] = if f.abs() >= 1.0 {
                libm::roundf(pow2(s2 as i32 - 1) / f).clamp(-32768.0, 32767.0) as i16
            } else {
                0
            };
            let rounding = if s1 > 0 { pow2(s1 as i32 - 1) } else { 0.0 };
            sums[j] = (bias_units[j] + rounding).clamp(-pow2(39), pow2(39) - 1.0) as i64;
            let remainder = bias[j] - bias_units[j] * scale[j];
            offset[j] = libm::roundf(remainder * gain[j] / step).clamp(-32768.0, 32767.0) as i16;
        }
        Self {
            image: encode_lanes(&sums),
            half,
            factor,
            offset,
            s1,
            s2,
            reserved: [0; 2],
        }
    }
}

/// `ceil(log2(x))` for a positive `x`, from its exponent bits: no
/// library call (`f64` and the logarithms are software routines on the
/// board, and the plans are made for every layer of every pass).
#[inline]
pub(super) fn ceil_log2(x: f32) -> i32 {
    debug_assert!(x > 0.0);
    let bits = x.to_bits();
    let exponent = ((bits >> 23) & 0xFF) as i32 - 127;
    let mantissa = bits & 0x7F_FFFF;
    if mantissa == 0 {
        exponent
    } else {
        exponent + 1
    }
}

/// `2^n` for `-126 <= n <= 127`.
#[inline]
pub(super) fn pow2(n: i32) -> f32 {
    f32::from_bits(((127 + n) as u32) << 23)
}

/// Eight 40-bit lane values as the accumulator stores and loads them:
/// lanes 0 to 3 in words 0..5 (160 bits), lanes 4 to 7 in words 8..13.
pub fn encode_lanes(sums: &[i64; LANES]) -> [u32; 16] {
    let mut words = [0u32; 16];
    for (half, base) in [0usize, 8].into_iter().enumerate() {
        // Five bytes per lane, little-endian, in twenty bytes.
        let mut bytes = [0u8; 20];
        for i in 0..4 {
            let lane = (sums[half * 4 + i] as u64).to_le_bytes();
            bytes[5 * i..5 * i + 5].copy_from_slice(&lane[..5]);
        }
        for (w, word) in bytes.chunks_exact(4).enumerate() {
            words[base + w] = u32::from_le_bytes([word[0], word[1], word[2], word[3]]);
        }
    }
    words
}

/// The eight 40-bit lane values of a stored accumulator, sign-extended.
#[inline]
pub fn decode_lanes(words: &[u32; 16]) -> [i64; LANES] {
    let mut sums = [0i64; LANES];
    for (half, base) in [0usize, 8].into_iter().enumerate() {
        let [w0, w1, w2, w3, w4] = [
            words[base],
            words[base + 1],
            words[base + 2],
            words[base + 3],
            words[base + 4],
        ];
        let lanes: [(u32, u8); 4] = [
            (w0, w1 as u8),
            ((w1 >> 8) | (w2 << 24), (w2 >> 8) as u8),
            ((w2 >> 16) | (w3 << 16), (w3 >> 16) as u8),
            ((w3 >> 24) | (w4 << 8), (w4 >> 24) as u8),
        ];
        for (i, (low, high)) in lanes.into_iter().enumerate() {
            sums[half * 4 + i] = (i64::from(high as i8) << 32) | i64::from(low);
        }
    }
    sums
}

/// What a kernel does with each finished group of eight values.
#[derive(Clone, Copy)]
pub enum Store<'t> {
    /// Write them.
    Write,
    /// Add them to what is there (saturating): the residual connection.
    Add,
    /// Write `max(0, value)`.
    Relu,
    /// Write GELU of them: the values are clamped to `-16..16` in units
    /// of `2^-10` and looked up in the table while they are still in the
    /// cache.
    Gelu(&'t GeluTable<'t>),
}

/// A weight for the lane kernels: packed by eight output channels (see
/// [`pack`]), with the per-channel plans already made.
pub struct LaneWeight<'a> {
    /// The `i8` weights, `[group][input][8]`.
    pub data: &'a [i8],
    /// Weights per output channel.
    pub per_output: usize,
    /// One plan per group of eight output channels.
    pub plans: &'a [GroupPlan],
}

/// One contiguous stretch of input that an output reads: the offset of
/// its first value, the offset of the matching weights inside a filter,
/// and its length (even, at least two). `repr(C)`: the assembly reads
/// the triples.
#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
pub struct Run {
    /// The offset of the first input value.
    pub input: usize,
    /// The offset of the first weight inside the filter.
    pub weight: usize,
    /// The number of values.
    pub len: usize,
}

/// The most runs one output pixel of a full convolution can have.
pub const MAX_RUNS: usize = 32;

/// Whether the vector unit is present.
pub const fn available() -> bool {
    cfg!(target_arch = "xtensa")
}

/// A linear layer on every row of `input` (`in_features` values each):
/// the output row `o` of channel `8 g + j` is the plan `g`'s epilogue of
/// `sum_k input[k] * w[8 g + j][k]`, written to `output` (rows of
/// `8 * plans.len()` values) as `store` says.
///
/// # Panics
///
/// When the shapes do not match, `in_features` is odd, or a buffer is
/// misaligned.
pub fn linear(
    input: &[i16],
    in_features: usize,
    weight: &LaneWeight<'_>,
    store: Store<'_>,
    output: &mut [i16],
) -> usize {
    assert!(
        in_features >= 2 && in_features.is_multiple_of(2),
        "lanes::linear in_features"
    );
    assert!(
        input.len().is_multiple_of(in_features),
        "lanes::linear input"
    );
    let rows = input.len() / in_features;
    let groups = weight.plans.len();
    let out_features = groups * LANES;
    assert_eq!(weight.per_output, in_features, "lanes::linear weight");
    assert!(
        weight.data.len() >= out_features * in_features,
        "lanes::linear weight data"
    );
    assert!(output.len() >= rows * out_features, "lanes::linear output");
    let run = [Run {
        input: 0,
        weight: 0,
        len: in_features,
    }];
    // Groups of filters a chunk at a time, every row against the chunk,
    // so the chunk's weights come from the cache after the first row.
    let chunk = (CACHED_WEIGHT_BYTES / (in_features * LANES)).clamp(1, GROUP_BATCH);
    for first in (0..groups).step_by(chunk) {
        let count = (groups - first).min(chunk);
        for (row, source) in input.chunks_exact(in_features).enumerate() {
            let out = &mut output[row * out_features + first * LANES..][..count * LANES];
            groups_epilogue(source, weight, first, count, &run, store, out);
        }
    }
    rows
}

/// The weights a chunk of filters may hold so that it stays in the data
/// cache while every row is run against it.
const CACHED_WEIGHT_BYTES: usize = 12 * 1024;

/// The most groups one assembly call handles.
pub const GROUP_BATCH: usize = 8;

/// A full convolution (square `kernel`, `stride`, symmetric `padding`
/// with zeros outside) on the channels-last `i16` input of `shape`; the
/// output has `8 * plans.len()` channels.
///
/// # Panics
///
/// When the shapes do not match.
#[allow(clippy::too_many_arguments)]
pub fn conv2d(
    input: &[i16],
    shape: Shape,
    weight: &LaneWeight<'_>,
    kernel: usize,
    stride: usize,
    padding: usize,
    store: Store<'_>,
    output: &mut [i16],
) -> Shape {
    let out_channels = weight.plans.len() * LANES;
    let out = Shape::new(
        output_size(shape.height, kernel, padding, stride),
        output_size(shape.width, kernel, padding, stride),
        out_channels,
    );
    assert_eq!(input.len(), shape.len(), "lanes::conv2d input");
    assert!(
        shape.channels >= 2 && shape.channels.is_multiple_of(2),
        "lanes::conv2d channels"
    );
    let taps = kernel * kernel * shape.channels;
    assert_eq!(weight.per_output, taps, "lanes::conv2d weight");
    assert!(
        weight.data.len() >= out_channels * taps,
        "lanes::conv2d weight data"
    );
    assert!(output.len() >= out.len(), "lanes::conv2d output");
    assert!(kernel * kernel <= MAX_RUNS, "lanes::conv2d kernel");
    let groups = weight.plans.len();
    let mut runs = [Run::default(); MAX_RUNS];
    for oy in 0..out.height {
        for ox in 0..out.width {
            let count = conv_runs(shape, kernel, stride, padding, ox, oy, &mut runs);
            let base = out.offset(ox, oy);
            for first in (0..groups).step_by(GROUP_BATCH) {
                let batch = (groups - first).min(GROUP_BATCH);
                let out = &mut output[base + first * LANES..][..batch * LANES];
                groups_epilogue(input, weight, first, batch, &runs[..count], store, out);
            }
        }
    }
    out
}

/// The runs of the output pixel `(ox, oy)` of a full convolution: a
/// kernel row inside the image is one run of `kernel * channels`; a
/// clipped row contributes one run of `channels` per tap inside.
#[inline(always)]
#[allow(clippy::too_many_arguments)]
fn conv_runs(
    shape: Shape,
    kernel: usize,
    stride: usize,
    padding: usize,
    ox: usize,
    oy: usize,
    runs: &mut [Run; MAX_RUNS],
) -> usize {
    let channels = shape.channels;
    let mut count = 0;
    let x_start = ox * stride;
    let row_inside = x_start >= padding && x_start - padding + kernel <= shape.width;
    for ky in 0..kernel {
        let Some(iy) = (oy * stride + ky).checked_sub(padding) else {
            continue;
        };
        if iy >= shape.height {
            continue;
        }
        if row_inside {
            runs[count] = Run {
                input: shape.offset(x_start - padding, iy),
                weight: ky * kernel * channels,
                len: kernel * channels,
            };
            count += 1;
            continue;
        }
        for kx in 0..kernel {
            let Some(ix) = (x_start + kx).checked_sub(padding) else {
                continue;
            };
            if ix >= shape.width {
                continue;
            }
            runs[count] = Run {
                input: shape.offset(ix, iy),
                weight: (ky * kernel + kx) * channels,
                len: channels,
            };
            count += 1;
        }
    }
    count
}

/// `count` groups of eight outputs from `first`, over `runs`, with their
/// epilogues, into `out` (`count * 8` values): the vector unit when it
/// is there, the scalar model otherwise.
fn groups_epilogue(
    input: &[i16],
    weight: &LaneWeight<'_>,
    first: usize,
    count: usize,
    runs: &[Run],
    store: Store<'_>,
    out: &mut [i16],
) {
    debug_assert_eq!(out.len(), count * LANES);
    #[cfg(target_arch = "xtensa")]
    {
        if arch::groups(input, weight, first, count, runs, store, out) {
            if let Store::Gelu(table) = store {
                table.apply(out);
            }
            return;
        }
    }
    model::groups(input, weight, first, count, runs, store, out);
    if let Store::Gelu(table) = store {
        table.apply(out);
    }
}

/// One tap of a depthwise convolution for one output pixel: the offset
/// of channel 0 of the input pixel it reads, and of channel 0 of its
/// weights. `repr(C)`: the assembly reads the pairs.
#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
pub struct Tap {
    /// The offset of the input pixel's first channel.
    pub input: usize,
    /// The offset of the tap's first weight.
    pub weight: usize,
}

/// The most taps a depthwise kernel may have (11 x 11).
pub const MAX_TAPS: usize = 121;

/// A depthwise convolution on the channels-last `i16` input of `shape`
/// (a multiple of eight channels), weights `[tap][channel]` as `i8`, one
/// plan per eight channels.
///
/// # Panics
///
/// When the shapes do not match.
#[allow(clippy::too_many_arguments)]
pub fn depthwise(
    input: &[i16],
    shape: Shape,
    weights: &[i8],
    plans: &[GroupPlan],
    kernel: usize,
    stride: usize,
    padding: usize,
    store: Store<'_>,
    output: &mut [i16],
) -> Shape {
    let channels = shape.channels;
    let out = Shape::new(
        output_size(shape.height, kernel, padding, stride),
        output_size(shape.width, kernel, padding, stride),
        channels,
    );
    assert_eq!(input.len(), shape.len(), "lanes::depthwise input");
    assert_eq!(channels, plans.len() * LANES, "lanes::depthwise channels");
    assert_eq!(
        weights.len(),
        kernel * kernel * channels,
        "lanes::depthwise weights"
    );
    assert!(output.len() >= out.len(), "lanes::depthwise output");
    assert!(kernel * kernel <= MAX_TAPS, "lanes::depthwise kernel");
    let mut taps = [Tap::default(); MAX_TAPS];
    for oy in 0..out.height {
        for ox in 0..out.width {
            let count = depthwise_taps(shape, kernel, stride, padding, ox, oy, &mut taps);
            let base = out.offset(ox, oy);
            let taps = &taps[..count];
            let out = &mut output[base..base + channels];
            #[cfg(target_arch = "xtensa")]
            {
                if arch::depthwise_pixel(input, weights, plans, taps, store, out) {
                    if let Store::Gelu(table) = store {
                        table.apply(out);
                    }
                    continue;
                }
            }
            model::depthwise_pixel(input, weights, plans, taps, store, out);
            if let Store::Gelu(table) = store {
                table.apply(out);
            }
        }
    }
    out
}

/// The taps of the output pixel `(ox, oy)` that lie inside the image.
#[inline(always)]
#[allow(clippy::too_many_arguments)]
fn depthwise_taps(
    shape: Shape,
    kernel: usize,
    stride: usize,
    padding: usize,
    ox: usize,
    oy: usize,
    taps: &mut [Tap; MAX_TAPS],
) -> usize {
    let mut count = 0;
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
            taps[count] = Tap {
                input: shape.offset(ix, iy),
                weight: (ky * kernel + kx) * shape.channels,
            };
            count += 1;
        }
    }
    count
}

/// The per-lane affine of a LayerNorm's output: `out = N * factor[j] >>
/// shift + offset[j]`, with `N` the standardized value in units of
/// `2^-11`; one per eight channels.
#[derive(Clone, Copy, Debug)]
#[repr(C, align(16))]
pub struct NormPlan {
    /// The factor of each lane.
    pub factor: [i16; LANES],
    /// Added after the shift, in output units.
    pub offset: [i16; LANES],
    /// The shift of the products.
    pub shift: u32,
    /// Padding.
    pub reserved: [u32; 3],
}

impl NormPlan {
    /// The plan for eight channels: `out = (N * gain[j] + bias[j]) /
    /// step[j]`.
    pub fn new(gain: &[f32; LANES], bias: &[f32; LANES], step: &[f32; LANES]) -> Self {
        let mut ratios = [0.0f32; LANES];
        let mut largest = 0.0f32;
        for j in 0..LANES {
            ratios[j] = gain[j] / step[j] / 2048.0;
            largest = largest.max(ratios[j].abs());
        }
        let shift = if largest > 0.0 {
            (14 - ceil_log2(largest)).clamp(0, 30) as u32
        } else {
            0
        };
        let mut factor = [0i16; LANES];
        let mut offset = [0i16; LANES];
        for j in 0..LANES {
            factor[j] =
                libm::roundf(ratios[j] * pow2(shift as i32)).clamp(-32768.0, 32767.0) as i16;
            offset[j] = libm::roundf(bias[j] / step[j]).clamp(-32768.0, 32767.0) as i16;
        }
        Self {
            factor,
            offset,
            shift,
            reserved: [0; 3],
        }
    }
}

/// The unit of a standardized value inside the LayerNorm: `2^-11`.
pub const NORM_UNIT_BITS: u32 = 11;

/// LayerNorm on every row of `channels` values of `input` (`i16`, one
/// mapping of step `step` for the whole tensor), written to `output`
/// through `plans` (one per eight channels): the standardized value of
/// each channel, times the plan's gain, plus its offset.
///
/// # Panics
///
/// When the lengths do not match.
pub fn layer_norm(
    input: &[i16],
    channels: usize,
    step: f32,
    epsilon: f32,
    plans: &[NormPlan],
    output: &mut [i16],
) {
    assert_eq!(channels, plans.len() * LANES, "lanes::layer_norm channels");
    assert!(
        channels > 0 && input.len().is_multiple_of(channels),
        "lanes::layer_norm input"
    );
    assert_eq!(input.len(), output.len(), "lanes::layer_norm output");
    for (row, out) in input
        .chunks_exact(channels)
        .zip(output.chunks_exact_mut(channels))
    {
        let (mean, scale, shift) = model::norm_row(row, step, epsilon);
        #[cfg(target_arch = "xtensa")]
        {
            if arch::norm_row(row, mean, scale, shift, plans, out) {
                continue;
            }
        }
        model::norm_apply(row, mean, scale, shift, plans, out);
    }
}

/// `target[i] = sat(target[i] + source[i])` on `i16`, in place.
///
/// # Panics
///
/// When the lengths differ.
pub fn add(target: &mut [i16], source: &[i16]) {
    assert_eq!(target.len(), source.len(), "lanes::add lengths");
    #[cfg(target_arch = "xtensa")]
    {
        if arch::add(target, source) {
            return;
        }
    }
    model::add(target, source);
}

/// `output[i] = sat(input[i] * factor >> shift)`: a tensor from one
/// mapping to another.
///
/// # Panics
///
/// When the lengths differ.
pub fn rescale(input: &[i16], factor: i16, shift: u32, output: &mut [i16]) {
    assert_eq!(input.len(), output.len(), "lanes::rescale lengths");
    #[cfg(target_arch = "xtensa")]
    {
        if arch::rescale(input, factor, shift, output) {
            return;
        }
    }
    model::rescale(input, factor, shift, output);
}

/// The factor and shift that take values of step `from` to step `to`:
/// `value * from / to`.
pub fn rescale_plan(from: f32, to: f32) -> (i16, u32) {
    let ratio = from / to;
    if ratio <= 0.0 {
        return (0, 0);
    }
    let shift = (14 - ceil_log2(ratio)).clamp(0, 30) as u32;
    let factor = libm::roundf(ratio * pow2(shift as i32)).clamp(0.0, 32767.0) as i16;
    (factor, shift)
}

/// 2x2 max pooling, stride 2, on a channels-last `i16` tensor.
///
/// # Panics
///
/// When the slices do not match the shapes.
pub fn max_pool_2x2(input: &[i16], shape: Shape, output: &mut [i16]) -> Shape {
    let out = Shape::new(shape.height / 2, shape.width / 2, shape.channels);
    assert_eq!(input.len(), shape.len(), "lanes::max_pool_2x2 input");
    assert!(output.len() >= out.len(), "lanes::max_pool_2x2 output");
    let channels = shape.channels;
    for oy in 0..out.height {
        for ox in 0..out.width {
            let a = shape.offset(ox * 2, oy * 2);
            let b = shape.offset(ox * 2, oy * 2 + 1);
            let o = out.offset(ox, oy);
            let (top, bottom) = (&input[a..a + 2 * channels], &input[b..b + 2 * channels]);
            let target = &mut output[o..o + channels];
            #[cfg(target_arch = "xtensa")]
            {
                if arch::max4(top, bottom, target) {
                    continue;
                }
            }
            model::max4(top, bottom, target);
        }
    }
    out
}

/// The dot product of two `i16` rows, exact.
///
/// # Panics
///
/// When the lengths differ.
pub fn dot(a: &[i16], b: &[i16]) -> i64 {
    assert_eq!(a.len(), b.len(), "lanes::dot lengths");
    #[cfg(target_arch = "xtensa")]
    {
        if let Some(sum) = arch::dot(a, b) {
            return sum;
        }
    }
    model::dot(a, b)
}

/// The attention's mixing for one output channel: `out[t] = sat(
/// (sum over r of weights[r] * rows[r * width + t]) * factor >> shift)`
/// for `t` in `0..width`, `width` a multiple of eight. `weights` are the
/// attention row (in units of `2^-15`), `rows` the value channels of a
/// head over the tokens.
///
/// # Panics
///
/// When the lengths do not match.
pub fn mix(rows: &[i16], width: usize, weights: &[i16], factor: i16, shift: u32, out: &mut [i16]) {
    assert!(
        width >= LANES && width.is_multiple_of(LANES),
        "lanes::mix width"
    );
    assert_eq!(rows.len(), weights.len() * width, "lanes::mix rows");
    assert_eq!(out.len(), width, "lanes::mix out");
    #[cfg(target_arch = "xtensa")]
    {
        if arch::mix(rows, width, weights, factor, shift, out) {
            return;
        }
    }
    model::mix(rows, width, weights, factor, shift, out);
}

/// The GELU lookup: `gelu(x)` for `x` in units of `2^-10`, clamped to
/// `-16..16`, in units of `2^-10`. Built once; 64 KB.
pub struct GeluTable<'a> {
    /// `gelu(x)` at `x = (i - 16384) / 1024`.
    table: &'a [i16],
}

impl<'a> GeluTable<'a> {
    /// Fraction bits of the input and the output.
    pub const UNIT_BITS: u32 = 10;
    /// Entries: `x` from -16 to 16, both included.
    pub const LEN: usize = (32 << Self::UNIT_BITS) + 1;

    /// A table built earlier into `storage` (see [`GeluTable::build`]).
    ///
    /// # Panics
    ///
    /// When `storage` is too short.
    pub fn borrow(storage: &'a [i16]) -> Self {
        assert!(storage.len() >= Self::LEN, "GELU table storage");
        Self {
            table: &storage[..Self::LEN],
        }
    }

    /// The built table's storage, to be borrowed again later.
    pub fn storage(&self) -> &'a [i16] {
        self.table
    }

    /// Build the table into `storage` (at least [`GeluTable::LEN`]
    /// values).
    ///
    /// # Panics
    ///
    /// When `storage` is too short.
    pub fn build(storage: &'a mut [i16]) -> Self {
        assert!(storage.len() >= Self::LEN, "GELU table storage");
        let table = &mut storage[..Self::LEN];
        let unit = (1u32 << Self::UNIT_BITS) as f32;
        for (i, slot) in table.iter_mut().enumerate() {
            let x = (i as f32 - (16 << Self::UNIT_BITS) as f32) / unit;
            let gelu = x * 0.5 * (1.0 + libm::erff(x * core::f32::consts::FRAC_1_SQRT_2));
            *slot = libm::roundf(gelu * unit).clamp(-32768.0, 32767.0) as i16;
        }
        Self { table }
    }

    /// Apply the table to every value of `data`, in place. The values
    /// must lie in `-16384..=16384` (the [`Store::Gelu`] store clamps
    /// them so).
    ///
    /// # Panics
    ///
    /// When a value is outside the table.
    #[inline]
    pub fn apply(&self, data: &mut [i16]) {
        let half = 16 << Self::UNIT_BITS;
        for value in data.iter_mut() {
            *value = self.table[(i32::from(*value) + half) as usize];
        }
    }
}

/// The step of a GELU table value: `2^-10`.
pub const GELU_STEP: f32 = 1.0 / (1 << GeluTable::UNIT_BITS) as f32;

/// The largest sum a layer can produce: products times the largest
/// input and weight, as a bound for [`GroupPlan::new`].
pub fn sum_bound(products: usize) -> f32 {
    products as f32 * 32767.0 * 127.0
}

/// Pack a plain `[output][per_output]` `i8` weight into the lane layout
/// (see [`pack::pack_rows`]).
pub fn pack_weight(rows: &[i8], outputs: usize, per_output: usize, packed: &mut [i8]) {
    pack::pack_rows(
        bytemuck::cast_slice(rows),
        outputs,
        per_output,
        bytemuck::cast_slice_mut(packed),
    );
}

/// The plans of a linear or full-convolution weight: `scales[o]` is the
/// weight's per-output scale, `in_step` the input's step, `bias[o]` the
/// bias, `out_step` the output's step, `products` the products per
/// output.
pub fn plan_groups(
    scales: &[f32],
    bias: &[f32],
    in_step: f32,
    out_step: f32,
    products: usize,
    plans: &mut [GroupPlan],
) {
    let outputs = scales.len();
    assert_eq!(bias.len(), outputs, "plan_groups bias");
    assert_eq!(plans.len() * LANES, outputs, "plan_groups groups");
    let bound = sum_bound(products);
    for (g, plan) in plans.iter_mut().enumerate() {
        let mut scale = [0.0f32; LANES];
        let mut b = [0.0f32; LANES];
        for j in 0..LANES {
            scale[j] = in_step * scales[g * LANES + j];
            b[j] = bias[g * LANES + j];
        }
        *plan = GroupPlan::new(&scale, &b, out_step, bound);
    }
}

/// The plans of a depthwise weight whose input has one step per channel
/// (`in_steps`), or one for all (`in_steps.len() == 1`).
pub fn plan_channels(
    scales: &[f32],
    bias: &[f32],
    in_steps: &[f32],
    out_step: f32,
    products: usize,
    plans: &mut [GroupPlan],
) {
    let channels = scales.len();
    assert_eq!(bias.len(), channels, "plan_channels bias");
    assert_eq!(plans.len() * LANES, channels, "plan_channels groups");
    let bound = sum_bound(products);
    for (g, plan) in plans.iter_mut().enumerate() {
        let mut scale = [0.0f32; LANES];
        let mut b = [0.0f32; LANES];
        for j in 0..LANES {
            let c = g * LANES + j;
            let in_step = if in_steps.len() == 1 {
                in_steps[0]
            } else {
                in_steps[c]
            };
            scale[j] = in_step * scales[c];
            b[j] = bias[c];
        }
        *plan = GroupPlan::new(&scale, &b, out_step, bound);
    }
}

#[cfg(target_arch = "xtensa")]
mod arch;
