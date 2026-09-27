//! EdgeFace-XXS on the vector unit's integer pipeline: the version the
//! board runs.
//!
//! The graph is the one of the parent module, block for block. Every
//! tensor between layers is `i16` with one symmetric mapping (see
//! [`lanes`]), and every layer runs in the vector registers from its
//! input to its output:
//!
//! - The residual stream `x` has one 16-bit mapping per stage
//!   (`q.stages.S.stream`), wide enough for the stream anywhere in the
//!   stage. A block's depthwise convolution reads it directly, and the
//!   MLP's second layer adds its result (times the block's `gamma`)
//!   straight into it.
//! - The depthwise output, the LayerNorm output, the MLP hidden tensor,
//!   the attention's tensors: each `i16` in its own per-tensor mapping,
//!   from the file (`q.<name>`) or fixed (the hidden tensor: units of
//!   `2^-10`, since GELU runs from a table in those units).
//! - Weights are `i8`, packed by eight output channels
//!   ([`pack`](super::super::pack)); the file's LayerNorm scale and shift
//!   are folded into the layer that follows, so the LayerNorm here only
//!   standardizes.
//! - The attention's matrix products are exact integer dot products; its
//!   softmax is `f32` on `per_head x per_head` values.
//!
//! The computer runs the same integer arithmetic in scalar code, bit for
//! bit, so `facekit` measures what the board computes.
//!
//! # Names in the weights file
//!
//! With `P` a block prefix such as `stages.2.blocks.5` and `S` a stage:
//!
//! | Name | The tensor |
//! | --- | --- |
//! | `stem.0` | the stem convolution's output, before its LayerNorm |
//! | `stages.S.stream` | the residual stream inside stage `S` |
//! | `P.conv_dw` | the depthwise output, as LayerNorm reads it |
//! | `P.norm` | the standardized LayerNorm input, as the folded `fc1` reads it |
//! | `stages.S.downsample.0` | the standardized downsample input |
//! | `P.convs.I.input` | the running sum of the split-convolution chain |
//! | `P.convs.I` | convolution `I`'s output |
//! | `P.tokens` | the token stream of a split-transpose block |
//! | `P.norm_xca` | the standardized attention input |
//! | `P.xca.qkv` | the packed queries, keys and values |
//! | `P.xca.mixed` | the attention output, as the projection reads it |
//! | `head.norm` | the standardized pooled vector |
//! | `embedding` | the embedding |
//!
//! Each is `[scale, 0]` (layout `Q16`). The image uses [`INPUT_QUANT`].

use super::{
    BlockName, EMBEDDING_LEN, HEADS, INPUT_SIZE, LAYER_NORM_EPSILON, MAX_ACTIVATION,
    NORMALIZE_EPSILON, Name, STAGE_CONV_BLOCKS, STAGE_KERNELS, STAGE_SPLIT_CONVS, weight,
};
use crate::nn::{
    Shape, Weights,
    lanes::{
        self, GELU_STEP, GeluTable, GroupPlan, LANES, LaneWeight, NormPlan, Store, model,
        plan_channels, plan_groups,
    },
    pack,
    quant::{Quant, simd},
    softmax_rows,
};

/// The mapping of the input image: `(byte / 255 - 0.5) / 0.5`, which lies
/// in -1..1, in steps of 1/127.
pub const INPUT_QUANT: Quant = Quant {
    scale: 1.0 / 127.0,
    zero_point: 0,
};
/// The widest MLP hidden tensor: 28x28 pixels of 4 x 24 values in stage 0.
const MAX_HIDDEN: usize = 28 * 28 * 96;
/// The most channels: stage 3.
const MAX_CHANNELS: usize = 168;
/// The most channels per attention head: stage 3.
const MAX_PER_HEAD: usize = MAX_CHANNELS / HEADS;
/// The widest packed `qkv` tensor: 196 tokens x 3 x 48 in stage 1.
const MAX_QKV: usize = 196 * 3 * 48;
/// The most values of one head's queries (or keys, or values) over the
/// tokens, padded to a multiple of eight: 12 channels x 200 in stage 1.
const MAX_HEAD_TOKENS: usize = 12 * 200;
/// The widest split-convolution chunk, padded to a multiple of eight
/// channels: 196 pixels x 24 channels.
const MAX_CHAIN: usize = 196 * 32;
/// The most groups of eight output channels of one layer: `fc1` of
/// stage 3, 672 outputs.
pub const MAX_GROUPS: usize = 672 / LANES;
/// The most LayerNorm groups: 168 channels.
pub const MAX_NORM_GROUPS: usize = MAX_CHANNELS / LANES;
/// The stem's input channels.
const STEM_CHANNELS: usize = 3;
/// The stem's input channels once padded for the vector unit.
const STEM_PADDED: usize = 8;
/// The stem's kernel (and stride).
const STEM_KERNEL: usize = 4;
/// The stem's output channels.
const STEM_FILTERS: usize = 24;
/// The largest attention matrix: 42 x 42 in stage 3.
const MAX_ATTENTION: usize = MAX_PER_HEAD * MAX_PER_HEAD;

/// The `i16` scratch one forward pass needs, in values.
pub const SCRATCH_I16_LEN: usize = 4 * MAX_ACTIVATION
    + MAX_HIDDEN
    + MAX_QKV
    + 3 * MAX_HEAD_TOKENS
    + MAX_HEAD_TOKENS
    + 2 * MAX_CHAIN;
/// The `f32` scratch one forward pass needs, in values: the attention
/// matrix and, when tracing, a dequantized copy of a block's output.
pub const SCRATCH_F32_LEN: usize = MAX_ATTENTION + MAX_ACTIVATION;
/// The group plans one forward pass needs.
pub const SCRATCH_PLANS: usize = MAX_GROUPS;
/// The LayerNorm plans one forward pass needs.
pub const SCRATCH_NORM_PLANS: usize = MAX_NORM_GROUPS;

/// The working buffers of a forward pass, borrowed from the caller. The
/// `i16` slice must start on a 16-byte boundary.
pub struct Scratch<'a> {
    /// The residual stream.
    x: &'a mut [i16],
    /// A block's depthwise output; the stem's widened input rows.
    a: &'a mut [i16],
    /// A block's standardized rows.
    b: &'a mut [i16],
    /// The token stream of a split-transpose block.
    tokens: &'a mut [i16],
    /// The MLP hidden tensor.
    hidden: &'a mut [i16],
    /// The packed queries, keys and values, token-major.
    qkv: &'a mut [i16],
    /// One head's queries, keys and values as rows over the tokens.
    heads: &'a mut [i16],
    /// One head's attention output as rows over the tokens.
    mixed: &'a mut [i16],
    /// The split-convolution chain's running sum and its convolution.
    chain: (&'a mut [i16], &'a mut [i16]),
    /// The attention matrix of one head; a traced block's values.
    f32s: &'a mut [f32],
    /// The group plans of the current layer.
    plans: &'a mut [GroupPlan],
    /// The LayerNorm plans of the current layer.
    norm_plans: &'a mut [NormPlan],
}

impl<'a> Scratch<'a> {
    /// Split the caller's memory.
    ///
    /// # Panics
    ///
    /// When a slice is too short or `i16s` is misaligned.
    pub fn new(
        i16s: &'a mut [i16],
        f32s: &'a mut [f32],
        plans: &'a mut [GroupPlan],
        norm_plans: &'a mut [NormPlan],
    ) -> Self {
        assert!(i16s.len() >= SCRATCH_I16_LEN, "EdgeFace i16 scratch");
        assert!(simd::aligned16(i16s), "EdgeFace i16 scratch alignment");
        assert!(f32s.len() >= SCRATCH_F32_LEN, "EdgeFace f32 scratch");
        assert!(plans.len() >= SCRATCH_PLANS, "EdgeFace plans");
        assert!(
            norm_plans.len() >= SCRATCH_NORM_PLANS,
            "EdgeFace norm plans"
        );
        let (x, rest) = i16s.split_at_mut(MAX_ACTIVATION);
        let (a, rest) = rest.split_at_mut(MAX_ACTIVATION);
        let (b, rest) = rest.split_at_mut(MAX_ACTIVATION);
        let (tokens, rest) = rest.split_at_mut(MAX_ACTIVATION);
        let (hidden, rest) = rest.split_at_mut(MAX_HIDDEN);
        let (qkv, rest) = rest.split_at_mut(MAX_QKV);
        let (heads, rest) = rest.split_at_mut(3 * MAX_HEAD_TOKENS);
        let (mixed, rest) = rest.split_at_mut(MAX_HEAD_TOKENS);
        let (chain0, rest) = rest.split_at_mut(MAX_CHAIN);
        let (chain1, _) = rest.split_at_mut(MAX_CHAIN);
        Self {
            x,
            a,
            b,
            tokens,
            hidden,
            qkv,
            heads,
            mixed,
            chain: (chain0, chain1),
            f32s,
            plans,
            norm_plans,
        }
    }
}

/// The step of the 16-bit mapping `q.<name>`.
fn step(weights: &impl Weights, name: &str) -> f32 {
    let pair = weights.get(Name::join("q", name).as_str());
    assert!(
        pair.len() >= 2 && pair[1] == 0.0,
        "{name}: a symmetric mapping"
    );
    pair[0]
}

/// The step of `q.prefix.suffix`.
fn step_of(weights: &impl Weights, prefix: &str, suffix: &str) -> f32 {
    step(weights, Name::join(prefix, suffix).as_str())
}

/// A packed weight with its scales and bias.
struct Packed<'w> {
    /// The packed `i8` values.
    data: &'w [i8],
    /// One scale per output channel.
    scales: &'w [f32],
    /// One bias per output channel.
    bias: &'w [f32],
}

/// The packed weight `prefix.suffix`.
fn packed<'w>(weights: &'w impl Weights, prefix: &str, suffix: &str) -> Packed<'w> {
    let base = Name::join(prefix, suffix);
    let weight_name = Name::join(base.as_str(), "weight");
    assert!(
        weights.packed(weight_name.as_str()),
        "{}: the lane kernels need weights packed by eight channels (nn::pack)",
        weight_name.as_str()
    );
    Packed {
        data: weights.get_i8(weight_name.as_str()),
        scales: weights.get(Name::join(base.as_str(), "scales").as_str()),
        bias: weights.get(Name::join(base.as_str(), "bias").as_str()),
    }
}

/// The lane weight of a packed linear or convolution weight, with its
/// plans made into `plans`: `in_step` the input's step, `out_step` the
/// output's, `gamma` an optional per-channel factor applied after the
/// layer (a block's layer scale) with the layer's own output range
/// before it, `per_output` the weights per output channel.
fn lane_weight<'p>(
    packed: &Packed<'p>,
    in_step: f32,
    out_step: f32,
    gamma: Option<(&[f32], f32)>,
    per_output: usize,
    plans: &'p mut [GroupPlan],
) -> LaneWeight<'p> {
    let outputs = packed.scales.len();
    let groups = outputs / LANES;
    assert_eq!(groups * LANES, outputs, "a multiple of eight outputs");
    let plans = &mut plans[..groups];
    match gamma {
        None => plan_groups(
            packed.scales,
            packed.bias,
            in_step,
            out_step,
            per_output,
            plans,
        ),
        Some((gamma, range)) => {
            for (g, plan) in plans.iter_mut().enumerate() {
                let mut scale = [0.0f32; LANES];
                let mut bias = [0.0f32; LANES];
                let mut gain = [0.0f32; LANES];
                for j in 0..LANES {
                    let o = g * LANES + j;
                    scale[j] = in_step * packed.scales[o];
                    bias[j] = packed.bias[o];
                    gain[j] = gamma[o];
                }
                *plan = GroupPlan::with_gain(
                    &scale,
                    &bias,
                    &gain,
                    range,
                    out_step,
                    lanes::sum_bound(per_output),
                );
            }
        }
    }
    LaneWeight {
        data: packed.data,
        per_output,
        plans,
    }
}

/// The plans of a plain standardization: gain 1, bias 0, into
/// `out_step`.
fn norm_plans(channels: usize, out_step: f32, plans: &mut [NormPlan]) -> &[NormPlan] {
    let groups = channels / LANES;
    let plans = &mut plans[..groups];
    for plan in plans.iter_mut() {
        *plan = NormPlan::new(&[1.0; LANES], &[0.0; LANES], &[out_step; LANES]);
    }
    plans
}

/// Run the recognizer on `input` (112x112x3, R, G, B, quantized with
/// [`INPUT_QUANT`]) and write the 512 raw embedding values. `gelu` is the
/// table built once by [`GeluTable::build`].
///
/// # Panics
///
/// When a buffer has the wrong size.
pub fn forward(
    weights: &impl Weights,
    gelu: &GeluTable<'_>,
    input: &[i8],
    scratch: Scratch<'_>,
    embedding: &mut [f32],
) {
    run(
        weights,
        gelu,
        input,
        scratch,
        embedding,
        None::<fn(&str, Shape, &[f32])>,
    );
}

/// [`forward`], calling `trace` after every block like the `f32`
/// version's `forward_traced`, with the same names and shapes (except the
/// head's LayerNorm output, which is folded away), each dequantized.
pub fn forward_traced(
    weights: &impl Weights,
    gelu: &GeluTable<'_>,
    input: &[i8],
    scratch: Scratch<'_>,
    embedding: &mut [f32],
    trace: impl FnMut(&str, Shape, &[f32]),
) {
    run(weights, gelu, input, scratch, embedding, Some(trace));
}

/// Dequantize `data` (step `step`) into the trace buffer and hand it to
/// `trace`, if there is one.
fn emit(
    trace: &mut Option<impl FnMut(&str, Shape, &[f32])>,
    buffer: &mut [f32],
    name: &str,
    shape: Shape,
    data: &[i16],
    step: f32,
) {
    let Some(trace) = trace else {
        return;
    };
    let values = &mut buffer[..shape.len()];
    for (real, &q) in values.iter_mut().zip(&data[..shape.len()]) {
        *real = f32::from(q) * step;
    }
    trace(name, shape, values);
}

/// The forward pass.
fn run(
    weights: &impl Weights,
    gelu: &GeluTable<'_>,
    input: &[i8],
    mut scratch: Scratch<'_>,
    embedding: &mut [f32],
    mut trace: Option<impl FnMut(&str, Shape, &[f32])>,
) {
    assert_eq!(
        input.len(),
        INPUT_SIZE * INPUT_SIZE * STEM_CHANNELS,
        "EdgeFace input"
    );
    assert!(
        embedding.len() >= EMBEDDING_LEN,
        "EdgeFace embedding buffer"
    );
    let f32s = core::mem::take(&mut scratch.f32s);
    let (attention, trace_buffer) = f32s.split_at_mut(MAX_ATTENTION);

    // Stem: Conv 4x4 stride 4 into `a`, then LayerNorm (with its own
    // scale and shift) into the stream.
    let mut stream_step = step(weights, "stages.0.stream");
    let mut shape = stem(weights, input, scratch.a, scratch.b, scratch.plans);
    {
        let plans = &mut scratch.norm_plans[..STEM_FILTERS / LANES];
        let gain = weights.get("stem.1.weight");
        let bias = weights.get("stem.1.bias");
        for (g, plan) in plans.iter_mut().enumerate() {
            let mut gains = [0.0f32; LANES];
            let mut biases = [0.0f32; LANES];
            for j in 0..LANES {
                gains[j] = gain[g * LANES + j];
                biases[j] = bias[g * LANES + j];
            }
            *plan = NormPlan::new(&gains, &biases, &[stream_step; LANES]);
        }
        lanes::layer_norm(
            &scratch.a[..shape.len()],
            shape.channels,
            step(weights, "stem.0"),
            LAYER_NORM_EPSILON,
            plans,
            &mut scratch.x[..shape.len()],
        );
    }
    emit(
        &mut trace,
        trace_buffer,
        "stem.1.Transpose_1",
        shape,
        scratch.x,
        stream_step,
    );

    for stage in 0..4 {
        if stage > 0 {
            let name = BlockName::downsample(stage);
            let next_step = step(weights, BlockName::stream(stage).as_str());
            shape = downsample(
                weights,
                name.as_str(),
                shape,
                stream_step,
                next_step,
                &mut scratch,
            );
            stream_step = next_step;
            emit(
                &mut trace,
                trace_buffer,
                Name::join(name.as_str(), "1.Conv").as_str(),
                shape,
                scratch.x,
                stream_step,
            );
        }
        for block in 0..STAGE_CONV_BLOCKS[stage] {
            let name = BlockName::block(stage, block);
            conv_block(
                weights,
                gelu,
                name.as_str(),
                STAGE_KERNELS[stage],
                shape,
                stream_step,
                &mut scratch,
            );
            emit(
                &mut trace,
                trace_buffer,
                Name::join(name.as_str(), "Add").as_str(),
                shape,
                scratch.x,
                stream_step,
            );
        }
        if stage > 0 {
            let name = BlockName::block(stage, STAGE_CONV_BLOCKS[stage]);
            split_transpose_block(
                weights,
                gelu,
                name.as_str(),
                STAGE_SPLIT_CONVS[stage - 1],
                stage == 1,
                shape,
                stream_step,
                &mut scratch,
                attention,
                &mut trace,
                trace_buffer,
            );
        }
    }

    // Head: mean over the pixels (exact), standardize, folded linear.
    let channels = shape.channels;
    let pooled = &mut scratch.a[..channels];
    {
        let pixels = shape.pixels() as i64;
        let mut sums = [0i64; MAX_CHANNELS];
        for pixel in scratch.x[..shape.len()].chunks_exact(channels) {
            for (sum, &v) in sums.iter_mut().zip(pixel) {
                *sum += i64::from(v);
            }
        }
        for (q, &sum) in pooled.iter_mut().zip(&sums[..channels]) {
            *q = model::sat16((2 * sum + pixels).div_euclid(2 * pixels));
        }
    }
    emit(
        &mut trace,
        trace_buffer,
        "head.global_pool.pool.GlobalAveragePool",
        Shape::new(1, 1, channels),
        pooled,
        stream_step,
    );
    let norm_step = step(weights, "head.norm");
    let normed = &mut scratch.b[..channels];
    lanes::layer_norm(
        pooled,
        channels,
        stream_step,
        LAYER_NORM_EPSILON,
        norm_plans(channels, norm_step, scratch.norm_plans),
        normed,
    );
    let embedding_step = step(weights, "embedding");
    let head = packed(weights, "head", "fc");
    let head = lane_weight(
        &head,
        norm_step,
        embedding_step,
        None,
        channels,
        scratch.plans,
    );
    let out = &mut scratch.hidden[..EMBEDDING_LEN];
    lanes::linear(normed, channels, &head, Store::Write, out);
    for (real, &q) in embedding[..EMBEDDING_LEN].iter_mut().zip(out.iter()) {
        *real = f32::from(q) * embedding_step;
    }
    if let Some(trace) = &mut trace {
        trace(
            "embedding",
            Shape::new(1, 1, EMBEDDING_LEN),
            &embedding[..EMBEDDING_LEN],
        );
    }
}

/// The stem convolution (4x4, stride 4, 3 -> 24 channels) into `out`, in
/// the `stem.0` mapping: the input widened to eight `i16` channels four
/// rows at a time (`band`), and the weights padded and packed to match.
fn stem(
    weights: &impl Weights,
    input: &[i8],
    out: &mut [i16],
    band: &mut [i16],
    plans: &mut [GroupPlan],
) -> Shape {
    const TAPS: usize = STEM_KERNEL * STEM_KERNEL;
    let narrow = weights.get_i8("stem.0.weight");
    let narrow_packed = weights.packed("stem.0.weight");
    assert_eq!(
        narrow.len(),
        STEM_FILTERS * TAPS * STEM_CHANNELS,
        "stem weight"
    );
    let at = |o: usize, k: usize| {
        if narrow_packed {
            narrow[pack::packed_index(o, k, TAPS * STEM_CHANNELS)]
        } else {
            narrow[o * TAPS * STEM_CHANNELS + k]
        }
    };
    // Padded rows, then packed.
    let mut rows = [0i8; STEM_FILTERS * TAPS * STEM_PADDED];
    for o in 0..STEM_FILTERS {
        for tap in 0..TAPS {
            for c in 0..STEM_CHANNELS {
                rows[(o * TAPS + tap) * STEM_PADDED + c] = at(o, tap * STEM_CHANNELS + c);
            }
        }
    }
    let mut packed_rows = simd::Aligned([0i8; STEM_FILTERS * TAPS * STEM_PADDED]);
    lanes::pack_weight(&rows, STEM_FILTERS, TAPS * STEM_PADDED, &mut packed_rows.0);
    let out_step = step(weights, "stem.0");
    let plans = &mut plans[..STEM_FILTERS / LANES];
    plan_groups(
        weights.get("stem.0.scales"),
        weights.get("stem.0.bias"),
        INPUT_QUANT.scale,
        out_step,
        TAPS * STEM_PADDED,
        plans,
    );
    let weight = LaneWeight {
        data: &packed_rows.0,
        per_output: TAPS * STEM_PADDED,
        plans,
    };

    let shape = Shape::new(
        INPUT_SIZE / STEM_KERNEL,
        INPUT_SIZE / STEM_KERNEL,
        STEM_FILTERS,
    );
    let band_shape = Shape::new(STEM_KERNEL, INPUT_SIZE, STEM_PADDED);
    let band = &mut band[..band_shape.len()];
    band.fill(0);
    let band_input = STEM_KERNEL * INPUT_SIZE * STEM_CHANNELS;
    for (oy, source) in input.chunks_exact(band_input).enumerate() {
        for (wide, narrow) in band
            .chunks_exact_mut(STEM_PADDED)
            .zip(source.chunks_exact(STEM_CHANNELS))
        {
            for (w, &n) in wide.iter_mut().zip(narrow) {
                *w = i16::from(n);
            }
        }
        let row = &mut out[oy * shape.width * STEM_FILTERS..(oy + 1) * shape.width * STEM_FILTERS];
        let produced = lanes::conv2d(
            band,
            band_shape,
            &weight,
            STEM_KERNEL,
            STEM_KERNEL,
            0,
            Store::Write,
            row,
        );
        debug_assert_eq!(produced, Shape::new(1, shape.width, STEM_FILTERS));
    }
    shape
}

/// Standardize the stream, then Conv 2x2 stride 2 with the folded
/// weights into the next stage's stream mapping. The result replaces
/// `scratch.x`.
fn downsample(
    weights: &impl Weights,
    prefix: &str,
    shape: Shape,
    stream_step: f32,
    next_step: f32,
    scratch: &mut Scratch<'_>,
) -> Shape {
    let norm_step = step_of(weights, prefix, "0");
    let normed = &mut scratch.a[..shape.len()];
    lanes::layer_norm(
        &scratch.x[..shape.len()],
        shape.channels,
        stream_step,
        LAYER_NORM_EPSILON,
        norm_plans(shape.channels, norm_step, scratch.norm_plans),
        normed,
    );
    let conv = packed(weights, prefix, "1");
    let conv = lane_weight(
        &conv,
        norm_step,
        next_step,
        None,
        4 * shape.channels,
        scratch.plans,
    );
    lanes::conv2d(normed, shape, &conv, 2, 2, 0, Store::Write, scratch.x)
}

/// `x += gamma * fc2(gelu(fc1(norm(depthwise(x)))))`.
#[allow(clippy::too_many_arguments)]
fn conv_block(
    weights: &impl Weights,
    gelu: &GeluTable<'_>,
    prefix: &str,
    kernel: usize,
    shape: Shape,
    stream_step: f32,
    scratch: &mut Scratch<'_>,
) {
    let len = shape.len();
    let channels = shape.channels;
    // The depthwise convolution reads the stream and writes `a` in its
    // own mapping.
    let dw_step = step_of(weights, prefix, "conv_dw");
    let dw = Name::join(prefix, "conv_dw");
    let dw_weight = weights.get_i8(Name::join(dw.as_str(), "weight").as_str());
    {
        let plans = &mut scratch.plans[..channels / LANES];
        plan_channels(
            weights.get(Name::join(dw.as_str(), "scales").as_str()),
            weights.get(Name::join(dw.as_str(), "bias").as_str()),
            &[stream_step],
            dw_step,
            kernel * kernel,
            plans,
        );
        lanes::depthwise(
            &scratch.x[..len],
            shape,
            dw_weight,
            plans,
            kernel,
            1,
            kernel / 2,
            Store::Write,
            &mut scratch.a[..len],
        );
    }
    // LayerNorm into `b`, then the MLP into the stream.
    let norm_step = step_of(weights, prefix, "norm");
    lanes::layer_norm(
        &scratch.a[..len],
        channels,
        dw_step,
        LAYER_NORM_EPSILON,
        norm_plans(channels, norm_step, scratch.norm_plans),
        &mut scratch.b[..len],
    );
    mlp(
        weights,
        gelu,
        prefix,
        &scratch.b[..len],
        norm_step,
        stream_step,
        &mut scratch.x[..len],
        scratch.hidden,
        scratch.plans,
    );
}

/// `stream += gamma * fc2(gelu(fc1(normed)))`: `normed` is the
/// standardized input (step `normed_step`) of the block `prefix`,
/// `stream` the tensor the result is added to (step `stream_step`).
#[allow(clippy::too_many_arguments)]
fn mlp(
    weights: &impl Weights,
    gelu: &GeluTable<'_>,
    prefix: &str,
    normed: &[i16],
    normed_step: f32,
    stream_step: f32,
    stream: &mut [i16],
    hidden: &mut [i16],
    plans: &mut [GroupPlan],
) {
    let fc1 = packed(weights, prefix, "mlp.fc1");
    let fc2 = packed(weights, prefix, "mlp.fc2");
    let channels = fc2.bias.len();
    let widened = fc1.bias.len();
    let rows = normed.len() / channels;
    let gamma = weight(weights, prefix, "gamma");
    let hidden = &mut hidden[..rows * widened];
    {
        let fc1 = lane_weight(&fc1, normed_step, GELU_STEP, None, channels, plans);
        lanes::linear(normed, channels, &fc1, Store::Gelu(gelu), hidden);
    }
    let fc2_range = step_of(weights, prefix, "mlp.fc2") * 32767.0;
    let fc2 = lane_weight(
        &fc2,
        GELU_STEP,
        stream_step,
        Some((gamma, fc2_range)),
        widened,
        plans,
    );
    lanes::linear(hidden, widened, &fc2, Store::Add, stream);
}

/// The split-convolution chain, the attention and the MLP of a
/// SplitTransposeBlock; see the `f32` version for the structure.
#[allow(clippy::too_many_arguments)]
fn split_transpose_block(
    weights: &impl Weights,
    gelu: &GeluTable<'_>,
    prefix: &str,
    convs: usize,
    positional: bool,
    shape: Shape,
    stream_step: f32,
    scratch: &mut Scratch<'_>,
    attention: &mut [f32],
    trace: &mut Option<impl FnMut(&str, Shape, &[f32])>,
    trace_buffer: &mut [f32],
) {
    let channels = shape.channels;
    let pixels = shape.pixels();
    let len = shape.len();
    let chunk = channels.div_ceil(convs + 1);
    let padded = chunk.div_ceil(LANES) * LANES;
    let chunk_shape = Shape::new(shape.height, shape.width, padded);
    let mut adds = 0;
    let add_name = |adds: usize| {
        const NAMES: [&str; 5] = ["Add", "Add_1", "Add_2", "Add_3", "Add_4"];
        NAMES[adds]
    };
    let tokens_step = step_of(weights, prefix, "tokens");

    // The chain, in one mapping wide enough for every sum and
    // convolution output in it, padded to a multiple of eight channels.
    let mut chain_step = stream_step;
    for index in 0..convs {
        let conv = Name::join("convs", ["0", "1", "2"][index]);
        let full = Name::join(prefix, conv.as_str());
        chain_step = chain_step
            .max(step_of(weights, full.as_str(), "input"))
            .max(step(weights, full.as_str()));
    }
    let (mut running, mut filtered) = (&mut *scratch.chain.0, &mut *scratch.chain.1);
    let (to_chain, to_chain_shift) = lanes::rescale_plan(stream_step, chain_step);
    let (to_tokens, to_tokens_shift) = lanes::rescale_plan(chain_step, tokens_step);
    let (stream_to_tokens, stream_to_tokens_shift) = lanes::rescale_plan(stream_step, tokens_step);
    let x = &scratch.x[..len];
    let tokens = &mut scratch.tokens[..len];
    for index in 0..convs {
        let range = index * chunk..(index + 1) * chunk;
        // The chunk of the stream, rescaled into the chain's mapping,
        // added to the running sum (or starting it).
        for (pixel, slot) in x
            .chunks_exact(channels)
            .zip(running.chunks_exact_mut(padded))
        {
            for (c, target) in slot.iter_mut().enumerate() {
                let value = if c < chunk {
                    model::scale16(pixel[range.start + c], to_chain, to_chain_shift)
                } else {
                    0
                };
                *target = if index == 0 {
                    value
                } else {
                    target.saturating_add(value)
                };
            }
        }
        if index > 0 {
            adds += 1;
            emit_chunk(
                trace,
                trace_buffer,
                Name::join(prefix, add_name(adds)).as_str(),
                shape,
                chunk,
                padded,
                &running[..pixels * padded],
                chain_step,
            );
        }
        // The convolution, weights padded to the chunk.
        let conv = Name::join("convs", ["0", "1", "2"][index]);
        let full = Name::join(prefix, conv.as_str());
        let dw_weight = weights.get_i8(Name::join(full.as_str(), "weight").as_str());
        let dw_scales = weights.get(Name::join(full.as_str(), "scales").as_str());
        let dw_bias = weights.get(Name::join(full.as_str(), "bias").as_str());
        let mut padded_weight = simd::Aligned([0i8; 9 * 48]);
        let mut padded_scales = [1.0f32; 48];
        let mut padded_bias = [0.0f32; 48];
        for tap in 0..9 {
            for c in 0..chunk {
                padded_weight.0[tap * padded + c] = dw_weight[tap * chunk + c];
            }
        }
        padded_scales[..chunk].copy_from_slice(&dw_scales[..chunk]);
        padded_bias[..chunk].copy_from_slice(&dw_bias[..chunk]);
        let plans = &mut scratch.plans[..padded / LANES];
        plan_channels(
            &padded_scales[..padded],
            &padded_bias[..padded],
            &[chain_step],
            chain_step,
            9,
            plans,
        );
        lanes::depthwise(
            &running[..pixels * padded],
            chunk_shape,
            &padded_weight.0[..9 * padded],
            plans,
            3,
            1,
            1,
            Store::Write,
            &mut filtered[..pixels * padded],
        );
        core::mem::swap(&mut running, &mut filtered);
        // Into the tokens, in their mapping.
        for (pixel, slot) in running
            .chunks_exact(padded)
            .zip(tokens.chunks_exact_mut(channels))
        {
            for c in 0..chunk {
                slot[range.start + c] = model::scale16(pixel[c], to_tokens, to_tokens_shift);
            }
        }
    }
    let last = convs * chunk..channels;
    for (pixel, slot) in x
        .chunks_exact(channels)
        .zip(tokens.chunks_exact_mut(channels))
    {
        for c in last.clone() {
            slot[c] = model::scale16(pixel[c], stream_to_tokens, stream_to_tokens_shift);
        }
    }

    if positional {
        // The positional constant, quantized into the tokens' mapping.
        let constant = weight(weights, prefix, "pos_embd.constant");
        let quantized = &mut scratch.b[..len];
        let inverse = 1.0 / tokens_step;
        for (q, &value) in quantized.iter_mut().zip(constant) {
            *q = Quant {
                scale: tokens_step,
                zero_point: 0,
            }
            .quantize16_with(inverse, value);
        }
        lanes::add(tokens, quantized);
        adds += 1;
        emit(
            trace,
            trace_buffer,
            Name::join(prefix, add_name(adds)).as_str(),
            shape,
            tokens,
            tokens_step,
        );
    }

    // Attention: standardize into `a`, qkv, the attention itself, the
    // projection added to the tokens.
    let norm_step = step_of(weights, prefix, "norm_xca");
    let normed = &mut scratch.a[..len];
    lanes::layer_norm(
        tokens,
        channels,
        tokens_step,
        LAYER_NORM_EPSILON,
        norm_plans(channels, norm_step, scratch.norm_plans),
        normed,
    );
    let qkv_step = step_of(weights, prefix, "xca.qkv");
    let packed_width = 3 * channels;
    let qkv = &mut scratch.qkv[..pixels * packed_width];
    {
        let qkv_weight = packed(weights, prefix, "xca.qkv");
        let qkv_weight = lane_weight(
            &qkv_weight,
            norm_step,
            qkv_step,
            None,
            channels,
            scratch.plans,
        );
        lanes::linear(normed, channels, &qkv_weight, Store::Write, qkv);
    }
    let mixed_step = step_of(weights, prefix, "xca.mixed");
    let mixed = &mut scratch.b[..len];
    cross_covariance_attention(
        weight(weights, prefix, "xca.temperature"),
        shape,
        qkv,
        qkv_step,
        scratch.heads,
        scratch.mixed,
        attention,
        mixed_step,
        mixed,
    );
    {
        let proj = packed(weights, prefix, "xca.proj");
        let proj_range = step_of(weights, prefix, "xca.proj") * 32767.0;
        let proj = lane_weight(
            &proj,
            mixed_step,
            tokens_step,
            Some((weight(weights, prefix, "gamma_xca"), proj_range)),
            channels,
            scratch.plans,
        );
        lanes::linear(mixed, channels, &proj, Store::Add, tokens);
    }
    adds += 1;
    emit(
        trace,
        trace_buffer,
        Name::join(prefix, add_name(adds)).as_str(),
        shape,
        tokens,
        tokens_step,
    );

    // The MLP on the tokens, added to the block's input.
    let mlp_norm_step = step_of(weights, prefix, "norm");
    let normed = &mut scratch.a[..len];
    lanes::layer_norm(
        tokens,
        channels,
        tokens_step,
        LAYER_NORM_EPSILON,
        norm_plans(channels, mlp_norm_step, scratch.norm_plans),
        normed,
    );
    mlp(
        weights,
        gelu,
        prefix,
        normed,
        mlp_norm_step,
        stream_step,
        &mut scratch.x[..len],
        scratch.hidden,
        scratch.plans,
    );
    adds += 1;
    emit(
        trace,
        trace_buffer,
        Name::join(prefix, add_name(adds)).as_str(),
        shape,
        scratch.x,
        stream_step,
    );
}

/// Trace a padded chain tensor: only its first `chunk` of `padded`
/// channels.
#[allow(clippy::too_many_arguments)]
fn emit_chunk(
    trace: &mut Option<impl FnMut(&str, Shape, &[f32])>,
    buffer: &mut [f32],
    name: &str,
    shape: Shape,
    chunk: usize,
    padded: usize,
    data: &[i16],
    step: f32,
) {
    let Some(trace) = trace else {
        return;
    };
    let chunk_shape = Shape::new(shape.height, shape.width, chunk);
    let values = &mut buffer[..chunk_shape.len()];
    for (pixel, slot) in data
        .chunks_exact(padded)
        .zip(values.chunks_exact_mut(chunk))
    {
        for (real, &q) in slot.iter_mut().zip(pixel) {
            *real = f32::from(q) * step;
        }
    }
    trace(name, chunk_shape, values);
}

/// Cross-covariance attention on the `i16` queries, keys and values
/// (token-major, `3 * channels` per token, one mapping), written as
/// `i16` (`mixed`, token-major) in the mapping of step `mixed_step`; see
/// the `f32` version. Per head, the queries, keys and values become rows
/// over the tokens (padded to a multiple of eight with zeros); the
/// covariance is exact integer dot products, the softmax `f32`, and the
/// mixing an integer weighted sum.
#[allow(clippy::too_many_arguments)]
pub fn cross_covariance_attention(
    temperature: &[f32],
    shape: Shape,
    qkv: &[i16],
    qkv_step: f32,
    heads: &mut [i16],
    mixed_rows: &mut [i16],
    attention: &mut [f32],
    mixed_step: f32,
    mixed: &mut [i16],
) {
    let (pixels, channels) = (shape.pixels(), shape.channels);
    let per_head = channels / HEADS;
    let packed = 3 * channels;
    let width = pixels.div_ceil(LANES) * LANES;
    let span = per_head * width;
    assert!(3 * span <= heads.len(), "attention head buffer");
    assert!(span <= mixed_rows.len(), "attention mixed buffer");
    let (queries, rest) = heads.split_at_mut(span);
    let (keys, values) = rest[..2 * span].split_at_mut(span);
    let attention = &mut attention[..per_head * per_head];
    let mixed_rows = &mut mixed_rows[..span];
    // The weighted sum of values (units 2^-15 * qkv_step) into the mixed
    // mapping.
    let (factor, shift) = lanes::rescale_plan(qkv_step, mixed_step);
    let square = qkv_step * qkv_step;

    for head in 0..HEADS {
        for (section, buffer) in [(0, &mut *queries), (1, &mut *keys), (2, &mut *values)] {
            buffer.fill(0);
            for c in 0..per_head {
                let column = section * channels + head * per_head + c;
                for (token, slot) in buffer[c * width..c * width + pixels].iter_mut().enumerate() {
                    *slot = qkv[token * packed + column];
                }
            }
        }
        // The inverse length of each query and key channel over the
        // tokens, in real units.
        let mut query_norms = [0.0f32; MAX_PER_HEAD];
        let mut key_norms = [0.0f32; MAX_PER_HEAD];
        for (norms, buffer) in [(&mut query_norms, &*queries), (&mut key_norms, &*keys)] {
            for (c, norm) in norms.iter_mut().enumerate().take(per_head) {
                let row = &buffer[c * width..(c + 1) * width];
                let squares = lanes::dot(row, row) as f32 * square;
                *norm = 1.0 / libm::sqrtf(squares).max(NORMALIZE_EPSILON);
            }
        }
        for c in 0..per_head {
            let query = &queries[c * width..(c + 1) * width];
            for c2 in 0..per_head {
                let key = &keys[c2 * width..(c2 + 1) * width];
                let covariance = lanes::dot(query, key) as f32 * square;
                attention[c * per_head + c2] =
                    covariance * query_norms[c] * key_norms[c2] * temperature[head];
            }
        }
        softmax_rows(attention, per_head);
        // The attention rows in units of 2^-15.
        let mut row_q = [0i16; MAX_PER_HEAD];
        for c in 0..per_head {
            for (q, &a) in row_q
                .iter_mut()
                .zip(&attention[c * per_head..(c + 1) * per_head])
            {
                *q = libm::roundf(a * 32767.0).clamp(0.0, 32767.0) as i16;
            }
            lanes::mix(
                values,
                width,
                &row_q[..per_head],
                factor,
                shift,
                &mut mixed_rows[c * width..(c + 1) * width],
            );
        }
        for c in 0..per_head {
            let row = &mixed_rows[c * width..c * width + pixels];
            for (token, &value) in row.iter().enumerate() {
                mixed[token * channels + head * per_head + c] = value;
            }
        }
    }
}
