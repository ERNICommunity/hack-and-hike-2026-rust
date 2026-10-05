//! Integer arithmetic shared by the two networks, and the detector's
//! kernels.
//!
//! An integer tensor stands for real values through a [`Quant`]:
//! `real = scale * (q - zero_point)`. Activations are 16-bit with one
//! symmetric mapping per tensor (zero point 0), chosen by `facekit
//! quantize` from the values seen on a calibration set. Weights are `i8`
//! with one scale per output channel and no zero point, so an output
//! value is
//!
//! ```text
//! out[o] = bias[o] + (in_scale * w_scale[o]) * sum(q_in * q_w)
//! ```
//!
//! with the sum exact. The kernels here ([`conv2d_16`], [`depthwise_16`])
//! accumulate on the vector unit and hand each sum to a store that writes
//! `f32` or requantizes to `i16` ([`Requant`]); the detector
//! ([`yunet::int8`](super::yunet::int8)) runs on them. The recognizer
//! runs on [`lanes`](super::lanes), where the requantization stays in the
//! vector registers as well; `docs/face_id/performance.md` has the
//! measurements behind the two designs.
//!
//! Also here: what `facekit quantize` needs to make the integer files
//! ([`Granularity`], [`quantize_weight_rows`],
//! [`quantize_weight_channels`], [`snr_db`]).
//!
//! Rounding is to the nearest integer, halves away from zero, and values
//! outside the integer range saturate. The same functions run on the
//! developer machine and on the board, so both see the same numbers.

use libm::roundf;

use super::{Activation, Shape, output_size, pack};

pub mod simd;

/// How an `i8` tensor maps to real values: `real = scale * (q - zero_point)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quant {
    /// The real value of one step.
    pub scale: f32,
    /// The `i8` value that stands for 0.0.
    pub zero_point: i32,
}

impl Quant {
    /// A mapping for values between `min` and `max`, using the full `i8`
    /// range. The range is widened to include 0.0, so that padding and
    /// ReLU are exact.
    pub fn from_range(min: f32, max: f32) -> Self {
        let (min, max) = (min.min(0.0), max.max(0.0));
        let scale = ((max - min) / 255.0).max(1e-8);
        let zero_point = roundf(-128.0 - min / scale).clamp(-128.0, 127.0) as i32;
        Self { scale, zero_point }
    }

    /// The mapping stored as two `f32` values, `[scale, zero point]`, in a
    /// weights file.
    pub fn from_pair(pair: &[f32]) -> Self {
        Self {
            scale: pair[0],
            zero_point: pair[1] as i32,
        }
    }

    /// A symmetric 16-bit mapping for values between `min` and `max`.
    pub fn from_range16(min: f32, max: f32) -> Self {
        let bound = min.abs().max(max.abs()).max(1e-8);
        Self {
            scale: bound / 32767.0,
            zero_point: 0,
        }
    }

    /// `1 / scale`: the quantizers multiply by it instead of dividing,
    /// which on the board is a single instruction instead of a routine.
    /// (`real * (1 / scale)` and `real / scale` can differ in their last
    /// bit, so a value exactly between two steps may round differently
    /// from a division; every quantizer here uses the product, on the
    /// computer as on the board.)
    #[inline]
    pub fn inverse_scale(&self) -> f32 {
        1.0 / self.scale
    }

    /// The `i16` value nearest to `real`.
    pub fn quantize16(&self, real: f32) -> i16 {
        self.quantize16_with(self.inverse_scale(), real)
    }

    /// [`Quant::quantize16`] with `1 / scale` computed once by the caller.
    #[inline]
    pub fn quantize16_with(&self, inverse_scale: f32, real: f32) -> i16 {
        (round_to_int(real * inverse_scale) + self.zero_point).clamp(-32768, 32767) as i16
    }

    /// The real value of the `i16` `q`.
    pub fn dequantize16(&self, q: i16) -> f32 {
        self.scale * (i32::from(q) - self.zero_point) as f32
    }

    /// The `i8` value nearest to `real`.
    pub fn quantize(&self, real: f32) -> i8 {
        self.quantize_with(self.inverse_scale(), real)
    }

    /// [`Quant::quantize`] with `1 / scale` computed once by the caller.
    #[inline]
    pub fn quantize_with(&self, inverse_scale: f32, real: f32) -> i8 {
        (round_to_int(real * inverse_scale) + self.zero_point).clamp(-128, 127) as i8
    }

    /// The real value of `q`.
    pub fn dequantize(&self, q: i8) -> f32 {
        self.scale * (i32::from(q) - self.zero_point) as f32
    }
}

/// `x` rounded to the nearest integer, halves away from zero, as
/// `libm::roundf` does, but inline: truncate (one instruction on the
/// board), then correct by the fraction. `x - trunc(x)` is exact for
/// every `f32` whose magnitude is below 2^23, and above that `x` is an
/// integer already. Out-of-range values saturate, as `as i32` does.
#[inline]
pub fn round_to_int(x: f32) -> i32 {
    let truncated = x as i32;
    let fraction = x - truncated as f32;
    // Branch-free: the two comparisons become conditional moves.
    let up = i32::from(fraction >= 0.5);
    let down = i32::from(fraction <= -0.5);
    truncated.saturating_add(up - down)
}

/// A sum as `f32`: through the 32-bit conversion instruction when it
/// fits, since the 64-bit conversion is a library routine on the board.
/// Both give the nearest `f32` of the same integer.
#[inline(always)]
pub fn sum_to_f32(sum: i64) -> f32 {
    let narrow = sum as i32;
    if i64::from(narrow) == sum {
        narrow as f32
    } else {
        wide_to_f32(sum)
    }
}

/// The 64-bit conversion, kept out of line so the compiler cannot hoist
/// its library call into the common path.
#[cold]
#[inline(never)]
fn wide_to_f32(sum: i64) -> f32 {
    sum as f32
}

/// How finely a tensor is quantized: one mapping for all of it, or one
/// per channel. The forward passes report it with every probe, so that
/// `facekit quantize` measures the right ranges.
///
/// Per-channel mappings are free for a depthwise convolution's input,
/// where channels never mix, and they matter: the residual stream of
/// EdgeFace-XXS, the recognizer before MFN_S8_V1, had a few channels a
/// hundred times larger than the rest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Granularity {
    /// One 8-bit mapping for the whole tensor.
    Tensor,
    /// One 8-bit mapping per channel; the tensor is channels-last with
    /// this many.
    Channel(usize),
    /// One 16-bit mapping for the whole tensor (symmetric, zero point 0).
    Wide,
    /// One 16-bit symmetric mapping per channel; the tensor is
    /// channels-last with this many. The residual stream of the
    /// integer recognizer, whose channels differ ten times in range.
    WideChannel(usize),
}

impl Granularity {
    /// The number of mappings: one, or one per channel.
    pub fn channels(self) -> usize {
        match self {
            Self::Tensor | Self::Wide => 1,
            Self::Channel(channels) | Self::WideChannel(channels) => channels,
        }
    }

    /// Whether the mappings are 16-bit.
    pub fn wide(self) -> bool {
        matches!(self, Self::Wide | Self::WideChannel(_))
    }
}

/// Where a kernel writes its result.
///
/// The integer output never passes through `f32`: the kernel turns each
/// exact integer sum into the output's integer with a [`Requant`], one
/// multiply and one shift.
pub enum Output<'a> {
    /// As `f32`.
    F32(&'a mut [f32]),
    /// As `i16`, quantized with the given mapping.
    I16(&'a mut [i16], Quant),
}

impl Output<'_> {
    /// The number of values the output has room for.
    fn len(&self) -> usize {
        match self {
            Output::F32(out) => out.len(),
            Output::I16(out, _) => out.len(),
        }
    }
}

/// The most output channels a kernel prepares constants for at once: a
/// full or depthwise convolution's channels (at most 64 in the
/// detector).
pub const SINK_WINDOW: usize = 192;

/// From an exact integer sum to an integer output: `real = bias + sum *
/// scale`, and the output is `round(real / step)`, computed as
/// `(sum * multiplier + offset) >> shift` with a 32-by-32-bit multiply
/// into 64 bits (two instructions on the board).
///
/// `multiplier` holds `scale / step` scaled by `2^shift` in 31 bits, so
/// a 32-bit sum times it stays inside 64 bits; `offset` holds the bias
/// and the rounding half. A sum outside `i32` (possible in principle for
/// a 672-wide row, never seen) takes a cold `f32` path. The same
/// integers are computed on the computer and on the board, so the
/// outputs agree bit for bit.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Requant {
    /// `scale / step`, scaled by `2^shift`.
    multiplier: i32,
    /// `bias / step` scaled by `2^shift`, plus half a unit for rounding.
    offset: i64,
    /// The binary point of the two above.
    shift: u32,
    /// `scale / step`, for the cold path.
    ratio: f32,
    /// `bias / step`, for the cold path.
    bias_steps: f32,
}

impl Requant {
    /// The most bits the products are shifted by; `bias / step` scaled by
    /// this stays inside `i64` for any bias the models have.
    const MAX_SHIFT: i32 = 40;

    /// The mapping of `bias + sum * scale` to units of `step`.
    ///
    /// # Panics
    ///
    /// When `scale / step` is 2^30 or more, or `bias / step` is 2^21 or
    /// more in magnitude (so that the products and the offset stay
    /// inside `i64`): no layer of the two models comes near either.
    pub fn new(scale: f32, bias: f32, step: f32) -> Self {
        let ratio = scale / step;
        let bias_steps = bias / step;
        assert!(
            ratio.abs() < 1_073_741_824.0 && bias_steps.abs() < 2_097_152.0,
            "requantizer: {ratio} steps per unit of sum, bias {bias_steps} steps"
        );
        // `ratio = m * 2^e` with `1 <= m < 2`; shift so that `ratio *
        // 2^shift` lies in `2^30..2^31`.
        let exponent = if ratio > 0.0 {
            ((ratio.to_bits() >> 23) & 0xFF) as i32 - 127
        } else {
            0
        };
        let shift = (30 - exponent).clamp(1, Self::MAX_SHIFT);
        let factor = f32::from_bits(((127 + shift) as u32) << 23);
        let multiplier = round_to_int(ratio * factor);
        let offset = roundf(bias_steps * factor) as i64 + (1i64 << (shift - 1));
        Self {
            multiplier,
            offset,
            shift: shift as u32,
            ratio,
            bias_steps,
        }
    }

    /// `round((bias + sum * scale) / step)`. The result is taken modulo
    /// `2^32`: an output beyond two billion steps of its mapping wraps
    /// instead of saturating, and no layer of the two models comes
    /// within a factor of ten thousand of that (a saturating version
    /// cost thirty instructions per value on the board).
    #[inline(always)]
    pub fn apply(&self, sum: i64) -> i32 {
        let narrow = sum as i32;
        if i64::from(narrow) == sum {
            ((i64::from(narrow) * i64::from(self.multiplier) + self.offset) >> self.shift) as i32
        } else {
            self.apply_wide(sum)
        }
    }

    /// The cold path for sums outside `i32`: the `f32` formula.
    #[cold]
    #[inline(never)]
    fn apply_wide(&self, sum: i64) -> i32 {
        round_to_int(self.bias_steps + sum as f32 * self.ratio)
    }
}

/// An [`Output`] as a kernel stores into it, with the constants of the
/// output channels prepared: the bias, one scale and, for the integer
/// output, one [`Requant`] per channel, computed once per kernel call
/// and read from the stack, not from the weights, per value. A kernel
/// stores a run of consecutive channels at once ([`Sink::store_all`]),
/// so the choice of output is made once per run and the loop over the
/// values is tight.
struct Sink<'s, 'a> {
    /// The output.
    output: &'s mut Output<'a>,
    /// The bias of every output channel.
    bias: &'s [f32],
    /// What happens to each value.
    activation: Activation,
    /// The first channel of the prepared window.
    first: usize,
    /// The channels in the window.
    count: usize,
    /// `in_scale * w_scale` of each channel of the window.
    scales: [f32; SINK_WINDOW],
    /// The bias of each channel of the window.
    biases: [f32; SINK_WINDOW],
    /// The requantizer of each channel of the window (integer output).
    requants: [Requant; SINK_WINDOW],
}

impl<'s, 'a> Sink<'s, 'a> {
    /// A sink for `output` with the bias of every channel.
    fn new(output: &'s mut Output<'a>, bias: &'s [f32], activation: Activation) -> Self {
        Self {
            output,
            bias,
            activation,
            first: 0,
            count: 0,
            scales: [0.0; SINK_WINDOW],
            biases: [0.0; SINK_WINDOW],
            requants: [Requant::default(); SINK_WINDOW],
        }
    }

    /// Prepare channels `first..first + count`; `scale_of(c)` is `in_scale
    /// * w_scale` of channel `c`.
    ///
    /// # Panics
    ///
    /// When the window is wider than [`SINK_WINDOW`].
    fn prepare(&mut self, first: usize, count: usize, scale_of: impl Fn(usize) -> f32) {
        assert!(
            count <= SINK_WINDOW,
            "a kernel prepares at most {SINK_WINDOW} output channels at once"
        );
        self.first = first;
        self.count = count;
        for i in 0..count {
            let c = first + i;
            let scale = scale_of(c);
            self.scales[i] = scale;
            self.biases[i] = self.bias[c];
            if let Output::I16(_, quant) = &*self.output {
                self.requants[i] = Requant::new(scale, self.bias[c], quant.scale);
            }
        }
    }

    /// Store the sums of output channels `first..first + sums.len()`
    /// (inside the prepared window) at `index..`.
    ///
    /// # Panics
    ///
    /// When the channels are outside the window.
    #[inline]
    fn store_all(&mut self, index: usize, first: usize, sums: &[i64]) {
        let n = sums.len();
        let i = first - self.first;
        debug_assert!(i + n <= self.count, "channels outside the prepared window");
        let scales = &self.scales[i..i + n];
        let biases = &self.biases[i..i + n];
        let requants = &self.requants[i..i + n];
        let activation = self.activation;
        match &mut *self.output {
            Output::F32(out) => {
                let out = &mut out[index..index + n];
                for (((o, &sum), &scale), &bias) in out.iter_mut().zip(sums).zip(scales).zip(biases)
                {
                    *o = activation.apply(bias + sum_to_f32(sum) * scale);
                }
            }
            Output::I16(out, quant) => {
                let out = &mut out[index..index + n];
                let zero = quant.zero_point;
                for ((o, &sum), requant) in out.iter_mut().zip(sums).zip(requants) {
                    let q = requant.apply(sum).saturating_add(zero);
                    *o = activation.apply_quantized(q, zero).clamp(-32768, 32767) as i16;
                }
            }
        }
    }

    /// Store the sum of output channel `channel` at `index`.
    #[inline(always)]
    fn store(&mut self, index: usize, channel: usize, sum: i64) {
        self.store_all(index, channel, &[sum]);
    }
}

impl Activation {
    /// Apply to a quantized value whose mapping has zero point `zero`:
    /// `max(0, x)` is `max(zero, q)`.
    #[inline(always)]
    fn apply_quantized(self, q: i32, zero: i32) -> i32 {
        match self {
            Self::None => q,
            Self::Relu => q.max(zero),
        }
    }
}

/// An integer weight tensor with its per-output-channel scales and its
/// `f32` bias.
#[derive(Clone, Copy)]
pub struct QWeight<'a> {
    /// The `i8` values, in the same layout as the `f32` weight would be,
    /// or grouped by eight output channels when `packed` (see
    /// `nn::pack`).
    pub data: &'a [i8],
    /// One real-value-per-step scale for each output channel.
    pub scales: &'a [f32],
    /// One bias per output channel.
    pub bias: &'a [f32],
    /// Whether `data` is grouped by eight output channels.
    pub packed: bool,
}

impl QWeight<'_> {
    /// Weight `k` of output channel `o`, whose filters have `per_output`
    /// weights each, in either layout.
    #[inline(always)]
    pub fn at(&self, o: usize, k: usize, per_output: usize) -> i8 {
        if self.packed {
            self.data[pack::packed_index(o, k, per_output)]
        } else {
            self.data[o * per_output + k]
        }
    }
}
/// An input element of the integer kernels: `i8` or `i16`.
pub trait Lane: Copy {
    /// The value as `i32`.
    fn widen(self) -> i32;

    /// Whether [`Lane::vector_dots`] exists for this type.
    const VECTOR_DOT: bool;

    /// The bytes of one value.
    const BYTES: usize;

    /// [`simd::dots_i16`] for this type; only when [`Lane::VECTOR_DOT`].
    fn vector_dots(
        input: &[Self],
        weights: &[i8],
        stride: usize,
        count: usize,
        sums: &mut [i64; simd::DOT_BATCH],
    ) -> bool;

    /// [`simd::group_sums_i16`] for this type; only when
    /// [`Lane::VECTOR_DOT`].
    fn vector_group_sums(
        input: &[Self],
        weights: &[i8],
        per_output: usize,
        first_group: usize,
        runs: &[simd::RunSpec],
        lanes: &mut [simd::Lanes],
    ) -> bool;

    /// Consecutive blocks of eight channels of a depthwise output pixel
    /// on the vector unit, from `channel`, one block per entry of
    /// `zeros` (the channels' zero points, which an `i16` input must not
    /// have) into `lanes`.
    fn vector_sums(
        pixel: &simd::DepthwisePixel<'_, Self>,
        zeros: &[simd::ZeroPoints],
        channel: usize,
        lanes: &mut [simd::Lanes],
    );
}

impl Lane for i8 {
    #[inline(always)]
    fn widen(self) -> i32 {
        i32::from(self)
    }

    const VECTOR_DOT: bool = false;

    const BYTES: usize = 1;

    fn vector_dots(
        _: &[Self],
        _: &[i8],
        _: usize,
        _: usize,
        _: &mut [i64; simd::DOT_BATCH],
    ) -> bool {
        false
    }

    fn vector_group_sums(
        _: &[Self],
        _: &[i8],
        _: usize,
        _: usize,
        _: &[simd::RunSpec],
        _: &mut [simd::Lanes],
    ) -> bool {
        false
    }

    fn vector_sums(
        pixel: &simd::DepthwisePixel<'_, Self>,
        zeros: &[simd::ZeroPoints],
        channel: usize,
        lanes: &mut [simd::Lanes],
    ) {
        pixel.sums(zeros, channel, lanes);
    }
}

impl Lane for i16 {
    #[inline(always)]
    fn widen(self) -> i32 {
        i32::from(self)
    }

    const VECTOR_DOT: bool = true;

    const BYTES: usize = 2;

    fn vector_dots(
        input: &[Self],
        weights: &[i8],
        stride: usize,
        count: usize,
        sums: &mut [i64; simd::DOT_BATCH],
    ) -> bool {
        simd::dots_i16(input, weights, stride, count, sums)
    }

    fn vector_group_sums(
        input: &[Self],
        weights: &[i8],
        per_output: usize,
        first_group: usize,
        runs: &[simd::RunSpec],
        lanes: &mut [simd::Lanes],
    ) -> bool {
        simd::group_sums_i16(input, weights, per_output, first_group, runs, lanes)
    }

    fn vector_sums(
        pixel: &simd::DepthwisePixel<'_, Self>,
        zeros: &[simd::ZeroPoints],
        channel: usize,
        lanes: &mut [simd::Lanes],
    ) {
        assert!(
            zeros.iter().all(|zero| zero.0.iter().all(|&z| z == 0)),
            "16-bit zero points"
        );
        pixel.sums(channel, &mut lanes[..zeros.len()]);
    }
}

// # How the kernels are shaped
//
// The board is an in-order core: a multiply that waits for a load, and
// an add that waits for the multiply, cost their full latencies. A loop
// with one running sum is a chain of such waits. The kernels below
// compute [`BLOCK`] outputs from one pass over the input instead: every
// input value is loaded once and multiplied into four independent sums,
// which gives the pipeline four chains to interleave and a quarter of the
// input loads. The sums are exact integers, so the result does not depend
// on this grouping: the blocked kernels give the same bits as a plain
// loop.
//
// A convolution reads its input in contiguous *runs*: a full convolution
// reads `in channels` values per tap, and when a kernel row lies inside
// the image its `kernel` taps are one run of `kernel * in channels`. A
// depthwise convolution reads one value per tap and channel, at a stride
// of `channels`, so it is grouped by channels instead: four channels at a
// time, taps inside.

/// Outputs computed from one pass over the input.
const BLOCK: usize = 4;

/// The dot products of `input` (each value minus `zero`) with four rows of
/// weights. The rows must be at least as long as the input; only the
/// first `input.len()` weights of each are used.
///
/// The sums are `i32`; the caller keeps the input short enough for them
/// (see [`WIDE_PRODUCTS`]).
#[inline(always)]
fn dot4<T: Lane>(input: &[T], zero: i32, rows: [&[i8]; BLOCK]) -> [i32; BLOCK] {
    let len = input.len();
    let (r0, r1, r2, r3) = (
        &rows[0][..len],
        &rows[1][..len],
        &rows[2][..len],
        &rows[3][..len],
    );
    let (mut s0, mut s1, mut s2, mut s3) = (0i32, 0i32, 0i32, 0i32);
    for ((((&v, &w0), &w1), &w2), &w3) in input.iter().zip(r0).zip(r1).zip(r2).zip(r3) {
        let v = v.widen() - zero;
        s0 += v * i32::from(w0);
        s1 += v * i32::from(w1);
        s2 += v * i32::from(w2);
        s3 += v * i32::from(w3);
    }
    [s0, s1, s2, s3]
}

/// The dot product of `input` (each value minus `zero`) with one row of
/// weights, for the outputs left over after the blocks of four.
#[inline(always)]
fn dot1<T: Lane>(input: &[T], zero: i32, row: &[i8]) -> i32 {
    let mut sum = 0i32;
    for (&v, &w) in input.iter().zip(&row[..input.len()]) {
        sum += (v.widen() - zero) * i32::from(w);
    }
    sum
}

/// The most 16-by-8-bit products an `i32` sum can hold in the worst case:
/// `516 * 32768 * 127 < i32::MAX`.
const WIDE_PRODUCTS: usize = 516;

/// A contiguous stretch of input that one output pixel of a full
/// convolution reads, and where the matching weights of a filter start.
#[derive(Clone, Copy, Default)]
struct Run {
    /// The offset of the first input value.
    input: usize,
    /// The offset of the first weight inside the filter.
    weight: usize,
    /// The number of values.
    len: usize,
}

/// The most runs one output pixel of a full convolution can have: one
/// per tap when every kernel row is clipped by the image edge.
const MAX_RUNS: usize = 32;

/// The runs of the output pixel `(ox, oy)` of a full convolution with a
/// square `kernel`, in `runs`; returns how many. A kernel row that lies
/// inside the image is one run of `kernel * channels`; a clipped row
/// contributes one run of `channels` per tap that is inside.
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

/// A full convolution over any input type; the public `conv2d` and
/// `conv2d_16` are this with the bounds of their sums checked.
#[allow(clippy::too_many_arguments)]
fn conv2d_blocked<T: Lane>(
    input: &[T],
    input_quant: Quant,
    shape: Shape,
    weight: &QWeight<'_>,
    kernel: usize,
    stride: usize,
    padding: usize,
    activation: Activation,
    output: &mut Output<'_>,
    name: &str,
) -> Shape {
    let out_channels = weight.bias.len();
    let out = Shape::new(
        output_size(shape.height, kernel, padding, stride),
        output_size(shape.width, kernel, padding, stride),
        out_channels,
    );
    assert_eq!(input.len(), shape.len(), "{name} input");
    let taps = kernel * kernel * shape.channels;
    assert_eq!(weight.data.len(), out_channels * taps, "{name} weight");
    assert_eq!(weight.scales.len(), out_channels, "{name} scales");
    assert!(output.len() >= out.len(), "{name} output buffer");
    assert!(kernel * kernel <= MAX_RUNS, "{name}: kernel too large");

    let zero = input_quant.zero_point;
    let filter = |o: usize| &weight.data[o * taps..(o + 1) * taps];
    let mut sink = Sink::new(output, weight.bias, activation);
    sink.prepare(0, out_channels, |o| input_quant.scale * weight.scales[o]);
    let mut runs = [Run::default(); MAX_RUNS];
    let mut specs = [simd::RunSpec::default(); MAX_RUNS];
    let mut lanes = [simd::Lanes([0; 16]); simd::GROUP_BLOCKS];
    let mut sums = [0i64; simd::DOT_BATCH];
    let mut partial = [0i64; simd::DOT_BATCH];
    let blocks = out_channels / BLOCK * BLOCK;
    if weight.packed {
        assert!(
            out_channels.is_multiple_of(pack::GROUP),
            "{name}: packed weights need a multiple of eight filters"
        );
    }
    // Packed weights go to the lane-parallel kernel: it needs a 16-bit
    // input without zero point, runs of an even length (a pixel is a
    // multiple of eight channels) and 8-byte aligned weights.
    let vector_packed = weight.packed
        && simd::available()
        && T::VECTOR_DOT
        && zero == 0
        && shape.channels.is_multiple_of(2)
        && simd::aligned8(weight.data);
    // The dot-product kernel needs every run 16-byte aligned in the
    // input and 8-byte aligned in the weights: a pixel is `channels *
    // BYTES` bytes and a filter row `channels` weights, so both follow
    // from the channel count once the buffers themselves are aligned.
    let vector = !weight.packed
        && simd::available()
        && T::VECTOR_DOT
        && zero == 0
        && (shape.channels * T::BYTES).is_multiple_of(16)
        && simd::aligned16(input)
        && simd::aligned8(weight.data);
    if !vector && !vector_packed {
        simd::note_fallback();
    }
    for oy in 0..out.height {
        for ox in 0..out.width {
            let count = conv_runs(shape, kernel, stride, padding, ox, oy, &mut runs);
            let runs = &runs[..count];
            let base = out.offset(ox, oy);
            if vector_packed {
                // Groups of eight filters in the accumulator lanes, a
                // batch of groups per assembly call, every run summed
                // inside it.
                for (spec, run) in specs.iter_mut().zip(runs) {
                    *spec = simd::RunSpec {
                        input: run.input,
                        weight: run.weight,
                        len: run.len,
                    };
                }
                let groups = out_channels / pack::GROUP;
                for first_group in (0..groups).step_by(simd::GROUP_BLOCKS) {
                    let batch = (groups - first_group).min(simd::GROUP_BLOCKS);
                    let ok = T::vector_group_sums(
                        input,
                        weight.data,
                        taps,
                        first_group,
                        &specs[..count],
                        &mut lanes[..batch],
                    );
                    assert!(ok, "checked vector operands");
                    for (b, block) in lanes[..batch].iter().enumerate() {
                        let o = (first_group + b) * pack::GROUP;
                        sink.store_all(base + o, o, &simd::decode_lanes(&block.0));
                    }
                }
                continue;
            }
            if weight.packed {
                // The scalar loop over packed weights, for the computer.
                for o in 0..out_channels {
                    let mut sum = 0i64;
                    for run in runs {
                        for (k, &v) in input[run.input..run.input + run.len].iter().enumerate() {
                            sum += i64::from(v.widen() - zero)
                                * i64::from(weight.at(o, run.weight + k, taps));
                        }
                    }
                    sink.store(base + o, o, sum);
                }
                continue;
            }
            if vector {
                // Batches of outputs: every run's dot products with the
                // batch's filters in one vector call, added up per output.
                for first in (0..out_channels).step_by(simd::DOT_BATCH) {
                    let count = (out_channels - first).min(simd::DOT_BATCH);
                    sums[..count].fill(0);
                    for run in runs {
                        let ok = T::vector_dots(
                            &input[run.input..run.input + run.len],
                            &weight.data[first * taps + run.weight..],
                            taps,
                            count,
                            &mut partial,
                        );
                        assert!(ok, "checked vector operands");
                        for (sum, &partial) in sums[..count].iter_mut().zip(&partial[..count]) {
                            *sum += partial;
                        }
                    }
                    sink.store_all(base + first, first, &sums[..count]);
                }
                continue;
            }
            for o in (0..blocks).step_by(BLOCK) {
                let filters = [filter(o), filter(o + 1), filter(o + 2), filter(o + 3)];
                let mut sums = [0i32; BLOCK];
                for run in runs {
                    let partial = dot4(
                        &input[run.input..run.input + run.len],
                        zero,
                        [
                            &filters[0][run.weight..],
                            &filters[1][run.weight..],
                            &filters[2][run.weight..],
                            &filters[3][run.weight..],
                        ],
                    );
                    for (sum, partial) in sums.iter_mut().zip(partial) {
                        *sum += partial;
                    }
                }
                sink.store_all(base + o, o, &sums.map(i64::from));
            }
            for o in blocks..out_channels {
                let filter = filter(o);
                let mut sum = 0i32;
                for run in runs {
                    sum += dot1(
                        &input[run.input..run.input + run.len],
                        zero,
                        &filter[run.weight..],
                    );
                }
                sink.store(base + o, o, i64::from(sum));
            }
        }
    }
    out
}

/// A full convolution on `i16` input; see [`conv2d`](super::conv2d).
///
/// The sum is `i32`: an output here has at most `kernel * kernel * in
/// channels` products, and the largest such count in the two models is
/// 2 x 2 x 88 = 352, whose worst-case sum fits. The assertion keeps that
/// true for any future model.
///
/// # Panics
///
/// When the slices do not match the shapes, or the kernel has more than
/// `WIDE_PRODUCTS` products per output.
#[allow(clippy::too_many_arguments)]
pub fn conv2d_16(
    input: &[i16],
    input_quant: Quant,
    shape: Shape,
    weight: &QWeight<'_>,
    kernel: usize,
    stride: usize,
    padding: usize,
    activation: Activation,
    mut output: Output<'_>,
) -> Shape {
    let taps = kernel * kernel * shape.channels;
    assert!(
        taps <= WIDE_PRODUCTS,
        "conv2d_16: {taps} products per output overflow an i32 sum"
    );
    conv2d_blocked(
        input,
        input_quant,
        shape,
        weight,
        kernel,
        stride,
        padding,
        activation,
        &mut output,
        "conv2d_16",
    )
}

/// One tap of a depthwise convolution for one output pixel: the offset of
/// channel 0 of the input pixel it reads, and of channel 0 of its weights.
/// `repr(C)` because the vector unit's assembly reads the pairs.
#[derive(Clone, Copy, Default)]
#[repr(C)]
pub struct Tap {
    /// The offset of the input pixel's first channel.
    pub input: usize,
    /// The offset of the tap's first weight.
    pub weight: usize,
}

/// The most taps a depthwise kernel may have (11 x 11).
const MAX_TAPS: usize = 121;

/// The taps of the output pixel `(ox, oy)` of a depthwise convolution that
/// lie inside the image, in `taps`; returns how many.
#[inline]
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

/// The sums of four neighbouring channels of a depthwise output pixel over
/// `taps`: `input[tap.input + c + i]` minus `zeros[i]`, times
/// `weights[tap.weight + c + i]`.
#[inline(always)]
fn depthwise_dot4<T: Lane>(
    input: &[T],
    zeros: [i32; BLOCK],
    weights: &[i8],
    c: usize,
    taps: &[Tap],
) -> [i32; BLOCK] {
    let (mut s0, mut s1, mut s2, mut s3) = (0i32, 0i32, 0i32, 0i32);
    for tap in taps {
        let [v0, v1, v2, v3] = *input[tap.input + c..]
            .first_chunk::<BLOCK>()
            .expect("four channels of input");
        let [w0, w1, w2, w3] = *weights[tap.weight + c..]
            .first_chunk::<BLOCK>()
            .expect("four channels of weights");
        s0 += (v0.widen() - zeros[0]) * i32::from(w0);
        s1 += (v1.widen() - zeros[1]) * i32::from(w1);
        s2 += (v2.widen() - zeros[2]) * i32::from(w2);
        s3 += (v3.widen() - zeros[3]) * i32::from(w3);
    }
    [s0, s1, s2, s3]
}

/// The sum of one channel of a depthwise output pixel over `taps`.
#[inline(always)]
fn depthwise_dot1<T: Lane>(input: &[T], zero: i32, weights: &[i8], c: usize, taps: &[Tap]) -> i32 {
    let mut sum = 0i32;
    for tap in taps {
        sum += (input[tap.input + c].widen() - zero) * i32::from(weights[tap.weight + c]);
    }
    sum
}

/// A depthwise convolution over any input type with any input mapping;
/// the public kernels are this with their sums' bounds checked.
#[allow(clippy::too_many_arguments)]
fn depthwise_blocked<T: Lane>(
    input: &[T],
    input_quant: impl Fn(usize) -> Quant,
    zero_free: bool,
    shape: Shape,
    weight: &QWeight<'_>,
    kernel: usize,
    stride: usize,
    padding: usize,
    activation: Activation,
    output: &mut Output<'_>,
    name: &str,
) -> Shape {
    let channels = shape.channels;
    let out = Shape::new(
        output_size(shape.height, kernel, padding, stride),
        output_size(shape.width, kernel, padding, stride),
        channels,
    );
    assert_eq!(input.len(), shape.len(), "{name} input");
    assert_eq!(
        weight.data.len(),
        kernel * kernel * channels,
        "{name} weight"
    );
    assert_eq!(weight.scales.len(), channels, "{name} scales");
    assert_eq!(weight.bias.len(), channels, "{name} bias");
    assert!(output.len() >= out.len(), "{name} output buffer");
    assert!(kernel * kernel <= MAX_TAPS, "{name}: kernel too large");

    let mut sink = Sink::new(output, weight.bias, activation);
    sink.prepare(0, channels, |c| input_quant(c).scale * weight.scales[c]);
    // The zero points of every channel, as the vector unit reads them
    // (eight per block) and as the scalar loops do.
    let mut zeros = [simd::ZeroPoints([0; simd::LANES]); SINK_WINDOW / simd::LANES];
    let mut zero_points = [0i32; SINK_WINDOW];
    for c in 0..channels {
        let zero = input_quant(c).zero_point;
        zero_points[c] = zero;
        zeros[c / simd::LANES].0[c % simd::LANES] = zero as i16;
    }
    let mut lanes = [simd::Lanes([0; 16]); simd::GROUP_BLOCKS];
    let mut taps = [Tap::default(); MAX_TAPS];
    let blocks = channels / BLOCK * BLOCK;
    // Eight channels at a time on the vector unit, when the channel count
    // is a multiple of eight (so every pixel's block of eight channels is
    // aligned); `DepthwisePixel::new` checks the buffers and every tap
    // once per pixel. A 16-bit input must have no zero point; an 8-bit
    // one has its zero points subtracted in the vector registers.
    let mut vector = simd::available() && channels.is_multiple_of(simd::LANES) && zero_free;
    for oy in 0..out.height {
        for ox in 0..out.width {
            let count = depthwise_taps(shape, kernel, stride, padding, ox, oy, &mut taps);
            let taps = &taps[..count];
            let base = out.offset(ox, oy);
            let mut c = 0;
            if vector {
                match simd::DepthwisePixel::new(input, weight.data, taps, channels) {
                    Some(pixel) => {
                        // Groups of blocks of eight channels: one assembly
                        // call per group, then each block decoded and
                        // stored.
                        while c < channels {
                            let blocks = ((channels - c) / simd::LANES).min(simd::GROUP_BLOCKS);
                            let first_block = c / simd::LANES;
                            T::vector_sums(
                                &pixel,
                                &zeros[first_block..first_block + blocks],
                                c,
                                &mut lanes[..blocks],
                            );
                            for block in &lanes[..blocks] {
                                let sums = simd::decode_lanes(&block.0);
                                sink.store_all(base + c, c, &sums);
                                c += simd::LANES;
                            }
                        }
                    }
                    None => {
                        // Misaligned or otherwise unsuitable: the scalar
                        // loops take over for the rest of the call.
                        vector = false;
                        simd::note_fallback();
                    }
                }
            }
            for c in (c / BLOCK * BLOCK..blocks).step_by(BLOCK) {
                let zeros = [
                    zero_points[c],
                    zero_points[c + 1],
                    zero_points[c + 2],
                    zero_points[c + 3],
                ];
                let sums = depthwise_dot4(input, zeros, weight.data, c, taps);
                sink.store_all(base + c, c, &sums.map(i64::from));
            }
            for (c, &zero) in zero_points.iter().enumerate().take(channels).skip(blocks) {
                let sum = depthwise_dot1(input, zero, weight.data, c, taps);
                sink.store(base + c, c, i64::from(sum));
            }
        }
    }
    out
}

/// A depthwise convolution on `i16` input with one mapping for the whole
/// tensor; see [`depthwise`](super::depthwise). The sum is `i32`: at most `kernel * kernel`
/// products per output, 121 at most, far inside the bound of
/// `WIDE_PRODUCTS`.
///
/// # Panics
///
/// When the slices do not match the shapes.
#[allow(clippy::too_many_arguments)]
pub fn depthwise_16(
    input: &[i16],
    input_quant: Quant,
    shape: Shape,
    weight: &QWeight<'_>,
    kernel: usize,
    stride: usize,
    padding: usize,
    activation: Activation,
    mut output: Output<'_>,
) -> Shape {
    const _: () = assert!(MAX_TAPS <= WIDE_PRODUCTS);
    depthwise_blocked(
        input,
        |_| input_quant,
        input_quant.zero_point == 0,
        shape,
        weight,
        kernel,
        stride,
        padding,
        activation,
        &mut output,
        "depthwise_16",
    )
}

/// The per-output-channel symmetric quantization of a weight tensor whose
/// output channel is the slowest dimension (`OHWI`, `OI`): `scales[o]` is
/// `max|w[o]| / 127`. `channel_len` is the number of values per output
/// channel.
///
/// # Panics
///
/// When the lengths do not match.
pub fn quantize_weight_rows(
    weight: &[f32],
    channel_len: usize,
    data: &mut [i8],
    scales: &mut [f32],
) {
    assert!(
        channel_len > 0 && weight.len().is_multiple_of(channel_len),
        "quantize_weight_rows length"
    );
    assert_eq!(weight.len(), data.len(), "quantize_weight_rows data");
    assert_eq!(
        scales.len(),
        weight.len() / channel_len,
        "quantize_weight_rows scales"
    );
    for ((row, out), scale) in weight
        .chunks_exact(channel_len)
        .zip(data.chunks_exact_mut(channel_len))
        .zip(scales)
    {
        let max = row
            .iter()
            .fold(0.0f32, |max, w| max.max(w.abs()))
            .max(1e-12);
        *scale = max / 127.0;
        for (q, w) in out.iter_mut().zip(row) {
            *q = roundf(w / *scale).clamp(-127.0, 127.0) as i8;
        }
    }
}

/// The per-channel symmetric quantization of a depthwise weight (`HWC`:
/// the channel is the fastest dimension). `channels` is the number of
/// channels.
///
/// # Panics
///
/// When the lengths do not match.
pub fn quantize_weight_channels(
    weight: &[f32],
    channels: usize,
    data: &mut [i8],
    scales: &mut [f32],
) {
    assert!(
        channels > 0 && weight.len().is_multiple_of(channels),
        "quantize_weight_channels length"
    );
    assert_eq!(weight.len(), data.len(), "quantize_weight_channels data");
    assert_eq!(scales.len(), channels, "quantize_weight_channels scales");
    for (c, scale) in scales.iter_mut().enumerate() {
        let max = weight
            .iter()
            .skip(c)
            .step_by(channels)
            .fold(0.0f32, |max, w| max.max(w.abs()))
            .max(1e-12);
        *scale = max / 127.0;
        for (q, w) in data
            .iter_mut()
            .skip(c)
            .step_by(channels)
            .zip(weight.iter().skip(c).step_by(channels))
        {
            *q = roundf(w / *scale).clamp(-127.0, 127.0) as i8;
        }
    }
}

/// The signal-to-quantization-noise ratio of `actual` against
/// `reference`, in decibels: `10 log10(sum(ref^2) / sum((ref - actual)^2))`.
/// Infinite when they are equal.
pub fn snr_db(reference: &[f32], actual: &[f32]) -> f32 {
    let mut signal = 0.0f32;
    let mut noise = 0.0f32;
    for (r, a) in reference.iter().zip(actual) {
        signal += r * r;
        noise += (r - a) * (r - a);
    }
    if noise == 0.0 {
        f32::INFINITY
    } else {
        10.0 * libm::log10f(signal / noise)
    }
}
