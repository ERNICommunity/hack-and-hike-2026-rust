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
//!   ([`pack`]); the file's LayerNorm scale and shift
//!   are folded into the layer that follows, so the LayerNorm here only
//!   standardizes. The stem's and stages 0 and 1's are also kept as
//!   `i16` in the compiled model (`WIDE_STAGES`): the same values,
//!   a shorter inner loop.
//! - The attention's matrix products are exact integer dot products; its
//!   softmax is `f32` on `per_head x per_head` values.
//!
//! The computer runs the same integer arithmetic in scalar code, bit for
//! bit, so `facekit` measures what the board computes.
//!
//! # Compile once, run many times
//!
//! Finding a tensor by its name walks the file's table of contents, and
//! the plan of a group of eight channels takes some forty divisions
//! (software routines on the board). One pass needs 277 tensors and 1,004
//! plans, and none of them depends on the image. So [`Model::compile`]
//! looks every tensor up and makes every plan once, into memory the
//! caller lends ([`ModelStorage`]), and [`Model::forward`] only computes.
//! The numbers are the same as when every pass made its own plans: the
//! same functions make them, from the same values.
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
    NORMALIZE_EPSILON, Name, STAGE_CHANNELS, STAGE_CONV_BLOCKS, STAGE_KERNELS, STAGE_SPLIT_CONVS,
    weight,
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
const MAX_GROUPS: usize = 672 / LANES;
/// The stem's input channels.
const STEM_CHANNELS: usize = 3;
/// The stem's input channels once padded for the vector unit.
const STEM_PADDED: usize = 8;
/// The stem's kernel (and stride).
const STEM_KERNEL: usize = 4;
/// The stem's taps: the pixels one output reads.
const STEM_TAPS: usize = STEM_KERNEL * STEM_KERNEL;
/// The stem's output channels.
const STEM_FILTERS: usize = 24;
/// The stem's weights once padded to [`STEM_PADDED`] channels.
const STEM_WEIGHTS: usize = STEM_FILTERS * STEM_TAPS * STEM_PADDED;
/// The largest attention matrix: 42 x 42 in stage 3.
const MAX_ATTENTION: usize = MAX_PER_HEAD * MAX_PER_HEAD;
/// The most ConvBlocks of one stage.
const MAX_CONV_BLOCKS: usize = 5;
/// The most split convolutions of one SplitTransposeBlock.
const MAX_SPLIT_CONVS: usize = 3;
/// The taps of a split convolution: 3 x 3.
const SPLIT_TAPS: usize = 9;
/// The positional encoding of stage 1: 14 x 14 tokens of 48 channels.
const POSITIONAL_LEN: usize = 14 * 14 * 48;
/// Every piece of a model's `i8` storage starts on a multiple of this, so
/// the vector unit's loads find it aligned.
const PIECE_ALIGN: usize = 16;
/// The stages whose linear and convolution weights the model also keeps
/// as `i16` (with the stem's): 0 and 1. The kernels then skip the
/// widening of every weight, which is most of their inner loop; these
/// weights are small (123 KB as `i16`) and each runs over 196 or 784
/// pixels, the later stages' are larger and run over 49 or 9.
const WIDE_STAGES: usize = 2;

/// The `i16` scratch one forward pass needs, in values. It includes the
/// widest MLP hidden tensor (`MAX_HIDDEN`, 147 KB), which goes unused when
/// [`Scratch::with_hidden`] lends a strip instead.
pub const SCRATCH_I16_LEN: usize = 4 * MAX_ACTIVATION
    + MAX_HIDDEN
    + MAX_QKV
    + 3 * MAX_HEAD_TOKENS
    + MAX_HEAD_TOKENS
    + 2 * MAX_CHAIN;
/// The `f32` scratch one forward pass needs, in values: the attention
/// matrix and, when tracing, a dequantized copy of a block's output.
pub const SCRATCH_F32_LEN: usize = MAX_ATTENTION + MAX_ACTIVATION;

/// What a compiled model needs of each kind of storage.
struct Needs {
    /// Group plans.
    plans: usize,
    /// LayerNorm plans.
    norm_plans: usize,
    /// Bytes of padded weights.
    weights: usize,
    /// `i16` weights of the wide layers.
    wide: usize,
}

/// A split-convolution chunk of `stage`, padded to a multiple of eight
/// channels.
const fn padded_chunk(stage: usize) -> usize {
    let chunk = STAGE_CHANNELS[stage].div_ceil(STAGE_SPLIT_CONVS[stage - 1] + 1);
    chunk.div_ceil(LANES) * LANES
}

/// Count what [`Model::compile`] takes from its storage, layer by layer
/// in the order of the graph.
const fn needs() -> Needs {
    // The stem and the head.
    let mut plans = STEM_FILTERS / LANES + EMBEDDING_LEN / LANES;
    let mut norm_plans = STEM_FILTERS / LANES + STAGE_CHANNELS[3] / LANES;
    let mut weights = STEM_WEIGHTS.div_ceil(PIECE_ALIGN) * PIECE_ALIGN;
    let mut wide = STEM_WEIGHTS;
    let mut stage = 0;
    while stage < STAGE_CHANNELS.len() {
        let groups = STAGE_CHANNELS[stage] / LANES;
        if stage < WIDE_STAGES {
            // `fc1` and `fc2` of each ConvBlock; the downsample, `qkv`,
            // the projection and the MLP of the SplitTransposeBlock.
            let c = STAGE_CHANNELS[stage];
            wide += STAGE_CONV_BLOCKS[stage] * 2 * 4 * c * c;
            if stage > 0 {
                wide += 4 * STAGE_CHANNELS[stage - 1] * c + (3 + 1 + 2 * 4) * c * c;
            }
        }
        // A ConvBlock: the depthwise convolution, `fc1` (four times as
        // wide) and `fc2`, and one LayerNorm.
        plans += STAGE_CONV_BLOCKS[stage] * (1 + 4 + 1) * groups;
        norm_plans += STAGE_CONV_BLOCKS[stage] * groups;
        if stage > 0 {
            // The downsample: a LayerNorm of the stage before, one
            // convolution.
            plans += groups;
            norm_plans += STAGE_CHANNELS[stage - 1] / LANES;
            // The SplitTransposeBlock: the split convolutions, `qkv`
            // (three times as wide), the projection, `fc1` and `fc2`, and
            // two LayerNorms.
            let convs = STAGE_SPLIT_CONVS[stage - 1];
            let padded = padded_chunk(stage);
            plans += convs * padded / LANES + (3 + 1 + 4 + 1) * groups;
            norm_plans += 2 * groups;
            weights += convs * (SPLIT_TAPS * padded).div_ceil(PIECE_ALIGN) * PIECE_ALIGN;
        }
        stage += 1;
    }
    Needs {
        plans,
        norm_plans,
        weights,
        wide,
    }
}

/// The group plans a compiled model holds: 1,004.
pub const MODEL_PLANS: usize = needs().plans;
/// The LayerNorm plans a compiled model holds: 208.
pub const MODEL_NORM_PLANS: usize = needs().norm_plans;
/// The bytes of padded weights a compiled model holds: the stem's and
/// the split convolutions'.
pub const MODEL_WEIGHTS_LEN: usize = needs().weights;
/// The `i16` constants a compiled model holds: the positional encoding
/// of stage 1 in the mapping of its tokens.
pub const MODEL_CONSTANTS_LEN: usize = POSITIONAL_LEN;
/// The `i16` weights a compiled model holds: the stem's and those of
/// stages 0 and 1, widened (62,976 values).
pub const MODEL_WIDE_LEN: usize = needs().wide;

/// The memory of a compiled model, lent by the caller for as long as the
/// model lives. The board keeps it in PSRAM.
pub struct ModelStorage<'m> {
    /// [`MODEL_PLANS`] group plans, with any values.
    pub plans: &'m mut [GroupPlan],
    /// [`MODEL_NORM_PLANS`] LayerNorm plans, with any values.
    pub norm_plans: &'m mut [NormPlan],
    /// [`MODEL_WEIGHTS_LEN`] bytes on a 16-byte boundary.
    pub weights: &'m mut [i8],
    /// [`MODEL_CONSTANTS_LEN`] values on a 16-byte boundary.
    pub constants: &'m mut [i16],
    /// [`MODEL_WIDE_LEN`] values on a 16-byte boundary.
    pub wide: &'m mut [i16],
}

impl<'m> ModelStorage<'m> {
    /// Check the lengths and the alignment.
    ///
    /// # Panics
    ///
    /// When a slice is too short or misaligned.
    fn check(&self) {
        assert!(self.plans.len() >= MODEL_PLANS, "EdgeFace model plans");
        assert!(
            self.norm_plans.len() >= MODEL_NORM_PLANS,
            "EdgeFace model norm plans"
        );
        assert!(
            self.weights.len() >= MODEL_WEIGHTS_LEN,
            "EdgeFace model weights"
        );
        assert!(
            self.constants.len() >= MODEL_CONSTANTS_LEN,
            "EdgeFace model constants"
        );
        assert!(self.wide.len() >= MODEL_WIDE_LEN, "EdgeFace model wide");
        assert!(
            simd::aligned16(self.weights)
                && simd::aligned16(self.constants)
                && simd::aligned16(self.wide),
            "EdgeFace model storage alignment"
        );
    }

    /// What is left of each kind of storage: plans, LayerNorm plans,
    /// weight bytes, constants and wide weights.
    fn lengths(&self) -> [usize; 5] {
        [
            self.plans.len(),
            self.norm_plans.len(),
            self.weights.len(),
            self.constants.len(),
            self.wide.len(),
        ]
    }

    /// Cut `count` group plans off the front.
    fn take_plans(&mut self, count: usize) -> &'m mut [GroupPlan] {
        take(&mut self.plans, count)
    }

    /// Cut `count` LayerNorm plans off the front.
    fn take_norm_plans(&mut self, count: usize) -> &'m mut [NormPlan] {
        take(&mut self.norm_plans, count)
    }

    /// Cut `len` bytes off the front, and what is left up to the next
    /// 16-byte boundary with them, so the next piece is aligned too.
    fn take_weights(&mut self, len: usize) -> &'m mut [i8] {
        let piece = take(&mut self.weights, len.div_ceil(PIECE_ALIGN) * PIECE_ALIGN);
        &mut piece[..len]
    }

    /// Cut `len` values off the front.
    fn take_constants(&mut self, len: usize) -> &'m mut [i16] {
        take(&mut self.constants, len)
    }

    /// Cut off room for `narrow` widened to `i16` and fill it. Every wide
    /// weight is a multiple of eight values long, so the next one starts
    /// on a 16-byte boundary too.
    fn take_wide(&mut self, narrow: &[i8]) -> &'m [i16] {
        debug_assert!(narrow.len().is_multiple_of(LANES));
        let wide = take(&mut self.wide, narrow.len());
        for (w, &n) in wide.iter_mut().zip(narrow) {
            *w = i16::from(n);
        }
        wide
    }
}

/// Cut the first `len` values off `rest`.
///
/// # Panics
///
/// When `rest` is shorter: the counts of [`needs`] and of
/// [`Model::compile`] disagree.
fn take<'a, T>(rest: &mut &'a mut [T], len: usize) -> &'a mut [T] {
    assert!(len <= rest.len(), "EdgeFace model storage used up");
    let (front, back) = core::mem::take(rest).split_at_mut(len);
    *rest = back;
    front
}

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
    /// The MLP hidden tensor, or a strip of it.
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
}

impl<'a> Scratch<'a> {
    /// Split the caller's memory.
    ///
    /// # Panics
    ///
    /// When a slice is too short or `i16s` is misaligned.
    pub fn new(i16s: &'a mut [i16], f32s: &'a mut [f32]) -> Self {
        assert!(i16s.len() >= SCRATCH_I16_LEN, "EdgeFace i16 scratch");
        assert!(simd::aligned16(i16s), "EdgeFace i16 scratch alignment");
        assert!(f32s.len() >= SCRATCH_F32_LEN, "EdgeFace f32 scratch");
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
        }
    }

    /// The same scratch with `hidden` for the MLP's hidden tensor: at
    /// least [`MIN_HIDDEN_LEN`] values on a 16-byte boundary. A shorter
    /// buffer than the widest tensor (`MAX_HIDDEN` values) is used for
    /// strips of rows, one after the other, with the same results. The
    /// board lends [`HIDDEN_STRIP_LEN`] values of internal RAM, where
    /// the kernels write and read the tensor without going to PSRAM.
    ///
    /// # Panics
    ///
    /// When `hidden` is too short or misaligned.
    pub fn with_hidden(self, hidden: &'a mut [i16]) -> Self {
        assert!(hidden.len() >= MIN_HIDDEN_LEN, "EdgeFace hidden strip");
        assert!(simd::aligned16(hidden), "EdgeFace hidden strip alignment");
        Self { hidden, ..self }
    }
}

/// The shortest buffer for [`Scratch::with_hidden`]: one row of the
/// widest MLP (672 values), which also holds the embedding.
pub const MIN_HIDDEN_LEN: usize = MAX_GROUPS * LANES;

/// The strip of the MLP's hidden tensor the board keeps in internal RAM
/// (36 KB): the whole tensor of stages 2 and 3, a third of stage 1's and a
/// fifth of stage 0's.
pub const HIDDEN_STRIP_LEN: usize = 18 * 1024;

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
/// plans made into `storage`: `in_step` the input's step, `out_step` the
/// output's, `gamma` an optional per-channel factor applied after the
/// layer (a block's layer scale) with the layer's own output range
/// before it, `per_output` the weights per output channel, `wide` whether
/// to keep an `i16` copy of the weights (`WIDE_STAGES`).
#[allow(clippy::too_many_arguments)]
fn lane_weight<'m>(
    packed: &Packed<'m>,
    in_step: f32,
    out_step: f32,
    gamma: Option<(&[f32], f32)>,
    per_output: usize,
    wide: bool,
    storage: &mut ModelStorage<'m>,
) -> LaneWeight<'m> {
    let outputs = packed.scales.len();
    let groups = outputs / LANES;
    assert_eq!(groups * LANES, outputs, "a multiple of eight outputs");
    let plans = storage.take_plans(groups);
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
    let data = &packed.data[..outputs * per_output];
    LaneWeight {
        data,
        wide: wide.then(|| storage.take_wide(data)),
        per_output,
        plans,
    }
}

/// A LayerNorm: the step of the tensor it reads, and the plans that
/// write the standardized values.
#[derive(Clone, Copy)]
struct Norm<'m> {
    /// The step of the input.
    input_step: f32,
    /// One plan per eight channels.
    plans: &'m [NormPlan],
}

impl<'m> Norm<'m> {
    /// A plain standardization of `channels` values of step `input_step`
    /// into `out_step`: gain 1, bias 0.
    fn plain(
        channels: usize,
        input_step: f32,
        out_step: f32,
        storage: &mut ModelStorage<'m>,
    ) -> Self {
        let plans = storage.take_norm_plans(channels / LANES);
        for plan in plans.iter_mut() {
            *plan = NormPlan::new(&[1.0; LANES], &[0.0; LANES], &[out_step; LANES]);
        }
        Self { input_step, plans }
    }

    /// Standardize every row of `input` into `output`.
    fn run(&self, input: &[i16], channels: usize, output: &mut [i16]) {
        lanes::layer_norm(
            input,
            channels,
            self.input_step,
            LAYER_NORM_EPSILON,
            self.plans,
            output,
        );
    }
}

/// The stem: Conv 4x4 stride 4, then LayerNorm with its own scale and
/// shift.
struct Stem<'m> {
    /// The convolution, its weights padded to eight input channels.
    conv: LaneWeight<'m>,
    /// The LayerNorm into the stream of stage 0.
    norm: Norm<'m>,
}

/// An MLP: `fc1` with GELU, then `fc2` times the block's `gamma`, added
/// to the stream.
struct Mlp<'m> {
    /// The first layer, into the GELU table's units.
    fc1: LaneWeight<'m>,
    /// The second layer, into the stream's mapping.
    fc2: LaneWeight<'m>,
    /// The channels of the input and of the stream.
    channels: usize,
    /// The width of the hidden tensor.
    widened: usize,
}

/// A ConvBlock.
struct ConvBlock<'m> {
    /// The depthwise weights, `[tap][channel]`.
    depthwise: &'m [i8],
    /// The depthwise plans, from the stream's mapping into its own.
    depthwise_plans: &'m [GroupPlan],
    /// The depthwise kernel.
    kernel: usize,
    /// The LayerNorm of the depthwise output.
    norm: Norm<'m>,
    /// The MLP.
    mlp: Mlp<'m>,
}

/// The downsample before a stage: LayerNorm, then Conv 2x2 stride 2.
struct Downsample<'m> {
    /// The LayerNorm of the stream of the stage before.
    norm: Norm<'m>,
    /// The convolution into the stream of this stage.
    conv: LaneWeight<'m>,
}

/// One convolution of a split-convolution chain.
#[derive(Clone, Copy)]
struct SplitConv<'m> {
    /// The weights, `[tap][channel]`, padded to a multiple of eight
    /// channels.
    weights: &'m [i8],
    /// The plans, inside the chain's mapping.
    plans: &'m [GroupPlan],
}

/// A factor and a shift that take a tensor from one mapping to another
/// (`lanes::rescale_plan`).
type Rescale = (i16, u32);

/// A SplitTransposeBlock.
struct AttentionBlock<'m> {
    /// The split convolutions, in order.
    convs: [Option<SplitConv<'m>>; MAX_SPLIT_CONVS],
    /// The channels of one chunk of the stream.
    chunk: usize,
    /// A chunk padded to a multiple of eight channels.
    padded: usize,
    /// The step of the chain's mapping.
    chain_step: f32,
    /// The step of the tokens' mapping.
    tokens_step: f32,
    /// From the stream's mapping to the chain's.
    to_chain: Rescale,
    /// From the chain's mapping to the tokens'.
    to_tokens: Rescale,
    /// From the stream's mapping to the tokens'.
    stream_to_tokens: Rescale,
    /// The positional encoding in the tokens' mapping (stage 1 only).
    positional: Option<&'m [i16]>,
    /// The LayerNorm before the attention.
    norm_xca: Norm<'m>,
    /// The layer that makes the queries, keys and values.
    qkv: LaneWeight<'m>,
    /// The step of the queries, keys and values.
    qkv_step: f32,
    /// The step of the attention's output.
    mixed_step: f32,
    /// The temperature of each head.
    temperature: &'m [f32],
    /// The projection, times `gamma_xca`, added to the tokens.
    proj: LaneWeight<'m>,
    /// The LayerNorm before the MLP.
    norm: Norm<'m>,
    /// The MLP.
    mlp: Mlp<'m>,
}

/// One stage.
struct Stage<'m> {
    /// The step of the residual stream inside the stage.
    stream_step: f32,
    /// The downsample before the stage; stage 0 has none.
    downsample: Option<Downsample<'m>>,
    /// The ConvBlocks, in order.
    blocks: [Option<ConvBlock<'m>>; MAX_CONV_BLOCKS],
    /// The SplitTransposeBlock after them; stage 0 has none.
    attention: Option<AttentionBlock<'m>>,
}

/// The head: the mean over the pixels, LayerNorm, the folded linear
/// layer.
struct Head<'m> {
    /// The LayerNorm of the pooled vector.
    norm: Norm<'m>,
    /// The linear layer.
    fc: LaneWeight<'m>,
    /// The step of the embedding.
    embedding_step: f32,
}

/// EdgeFace-XXS with every tensor found and every plan made: what
/// [`Model::forward`] needs besides the image.
///
/// Compile it once, when the application starts, and keep it. It borrows
/// the weights and the storage, so both must live as long as it does.
pub struct Model<'m> {
    /// The stem.
    stem: Stem<'m>,
    /// The four stages.
    stages: [Stage<'m>; 4],
    /// The head.
    head: Head<'m>,
}

impl<'m> Model<'m> {
    /// Look up every tensor of `weights` and make every plan, into
    /// `storage`.
    ///
    /// # Panics
    ///
    /// When a tensor is missing, a linear or convolution weight is not
    /// packed by eight channels (`nn::pack`), or `storage` is too small
    /// or misaligned.
    pub fn compile(weights: &'m impl Weights, mut storage: ModelStorage<'m>) -> Self {
        storage.check();
        let lent = storage.lengths();
        let stem = compile_stem(weights, &mut storage);
        let mut stream_step = step(weights, "stages.0.stream");
        let stages = core::array::from_fn(|stage| {
            let mut downsample = None;
            if stage > 0 {
                let name = BlockName::downsample(stage);
                let next_step = step(weights, BlockName::stream(stage).as_str());
                downsample = Some(compile_downsample(
                    weights,
                    name.as_str(),
                    STAGE_CHANNELS[stage - 1],
                    stream_step,
                    next_step,
                    stage < WIDE_STAGES,
                    &mut storage,
                ));
                stream_step = next_step;
            }
            let blocks = core::array::from_fn(|block| {
                (block < STAGE_CONV_BLOCKS[stage]).then(|| {
                    compile_conv_block(
                        weights,
                        BlockName::block(stage, block).as_str(),
                        STAGE_KERNELS[stage],
                        STAGE_CHANNELS[stage],
                        stream_step,
                        stage < WIDE_STAGES,
                        &mut storage,
                    )
                })
            });
            let attention = (stage > 0).then(|| {
                compile_attention_block(
                    weights,
                    BlockName::block(stage, STAGE_CONV_BLOCKS[stage]).as_str(),
                    STAGE_SPLIT_CONVS[stage - 1],
                    stage == 1,
                    Shape::new(
                        STAGE_SIDES[stage],
                        STAGE_SIDES[stage],
                        STAGE_CHANNELS[stage],
                    ),
                    stream_step,
                    stage < WIDE_STAGES,
                    &mut storage,
                )
            });
            Stage {
                stream_step,
                downsample,
                blocks,
                attention,
            }
        });
        let head = compile_head(weights, STAGE_CHANNELS[3], stream_step, &mut storage);
        let left = storage.lengths();
        debug_assert_eq!(
            core::array::from_fn::<usize, 5, _>(|kind| lent[kind] - left[kind]),
            [
                MODEL_PLANS,
                MODEL_NORM_PLANS,
                MODEL_WEIGHTS_LEN,
                MODEL_CONSTANTS_LEN,
                MODEL_WIDE_LEN
            ],
            "`needs` and Model::compile count different storage"
        );
        Self { stem, stages, head }
    }

    /// Run the recognizer on `input` (112x112x3, R, G, B, quantized with
    /// [`INPUT_QUANT`]) and write the 512 raw embedding values. `gelu` is
    /// the table built once by [`GeluTable::build`].
    ///
    /// # Panics
    ///
    /// When a buffer has the wrong size.
    pub fn forward(
        &self,
        gelu: &GeluTable<'_>,
        input: &[i8],
        scratch: Scratch<'_>,
        embedding: &mut [f32],
    ) {
        self.run(
            gelu,
            input,
            scratch,
            embedding,
            None::<fn(&str, Shape, &[f32])>,
        );
    }

    /// [`Model::forward`], calling `trace` after every block like the
    /// `f32` version's `forward_traced`, with the same names and shapes
    /// (except the head's LayerNorm output, which is folded away), each
    /// dequantized.
    pub fn forward_traced(
        &self,
        gelu: &GeluTable<'_>,
        input: &[i8],
        scratch: Scratch<'_>,
        embedding: &mut [f32],
        trace: impl FnMut(&str, Shape, &[f32]),
    ) {
        self.run(gelu, input, scratch, embedding, Some(trace));
    }

    /// The forward pass.
    fn run(
        &self,
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
        let mut shape = run_stem(&self.stem, input, scratch.a, scratch.b);
        self.stem.norm.run(
            &scratch.a[..shape.len()],
            shape.channels,
            &mut scratch.x[..shape.len()],
        );
        emit(
            &mut trace,
            trace_buffer,
            "stem.1.Transpose_1",
            shape,
            scratch.x,
            self.stages[0].stream_step,
        );

        for (index, stage) in self.stages.iter().enumerate() {
            if let Some(downsample) = &stage.downsample {
                shape = run_downsample(downsample, shape, &mut scratch);
                emit(
                    &mut trace,
                    trace_buffer,
                    Name::join(BlockName::downsample(index).as_str(), "1.Conv").as_str(),
                    shape,
                    scratch.x,
                    stage.stream_step,
                );
            }
            for (block, conv_block) in stage.blocks.iter().flatten().enumerate() {
                run_conv_block(conv_block, gelu, shape, &mut scratch);
                emit(
                    &mut trace,
                    trace_buffer,
                    Name::join(BlockName::block(index, block).as_str(), "Add").as_str(),
                    shape,
                    scratch.x,
                    stage.stream_step,
                );
            }
            if let Some(block) = &stage.attention {
                run_attention_block(
                    block,
                    gelu,
                    BlockName::block(index, STAGE_CONV_BLOCKS[index]).as_str(),
                    shape,
                    stage.stream_step,
                    &mut scratch,
                    attention,
                    &mut trace,
                    trace_buffer,
                );
            }
        }

        // Head: mean over the pixels (exact), standardize, folded linear.
        let stream_step = self.stages[3].stream_step;
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
        let normed = &mut scratch.b[..channels];
        self.head.norm.run(pooled, channels, normed);
        let out = &mut scratch.hidden[..EMBEDDING_LEN];
        lanes::linear(normed, channels, &self.head.fc, Store::Write, out);
        for (real, &q) in embedding[..EMBEDDING_LEN].iter_mut().zip(out.iter()) {
            *real = f32::from(q) * self.head.embedding_step;
        }
        if let Some(trace) = &mut trace {
            trace(
                "embedding",
                Shape::new(1, 1, EMBEDDING_LEN),
                &embedding[..EMBEDDING_LEN],
            );
        }
    }
}

/// The rows (and columns) of the map of each stage: 112 / 4, then halved
/// by each downsample (rounded down).
const STAGE_SIDES: [usize; 4] = [28, 14, 7, 3];

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

/// The stem's convolution with its weights padded to eight input
/// channels and packed, and its LayerNorm, which has its own scale and
/// shift.
fn compile_stem<'m>(weights: &'m impl Weights, storage: &mut ModelStorage<'m>) -> Stem<'m> {
    let narrow = weights.get_i8("stem.0.weight");
    let narrow_packed = weights.packed("stem.0.weight");
    assert_eq!(
        narrow.len(),
        STEM_FILTERS * STEM_TAPS * STEM_CHANNELS,
        "stem weight"
    );
    let at = |o: usize, k: usize| {
        if narrow_packed {
            narrow[pack::packed_index(o, k, STEM_TAPS * STEM_CHANNELS)]
        } else {
            narrow[o * STEM_TAPS * STEM_CHANNELS + k]
        }
    };
    // Padded rows, then packed.
    let mut rows = [0i8; STEM_WEIGHTS];
    for o in 0..STEM_FILTERS {
        for tap in 0..STEM_TAPS {
            for c in 0..STEM_CHANNELS {
                rows[(o * STEM_TAPS + tap) * STEM_PADDED + c] = at(o, tap * STEM_CHANNELS + c);
            }
        }
    }
    let packed_rows = storage.take_weights(STEM_WEIGHTS);
    lanes::pack_weight(&rows, STEM_FILTERS, STEM_TAPS * STEM_PADDED, packed_rows);
    let conv_step = step(weights, "stem.0");
    let plans = storage.take_plans(STEM_FILTERS / LANES);
    plan_groups(
        weights.get("stem.0.scales"),
        weights.get("stem.0.bias"),
        INPUT_QUANT.scale,
        conv_step,
        STEM_TAPS * STEM_PADDED,
        plans,
    );
    let conv = LaneWeight {
        data: packed_rows,
        wide: Some(storage.take_wide(packed_rows)),
        per_output: STEM_TAPS * STEM_PADDED,
        plans,
    };

    let stream_step = step(weights, "stages.0.stream");
    let norm_plans = storage.take_norm_plans(STEM_FILTERS / LANES);
    let gain = weights.get("stem.1.weight");
    let bias = weights.get("stem.1.bias");
    for (g, plan) in norm_plans.iter_mut().enumerate() {
        let mut gains = [0.0f32; LANES];
        let mut biases = [0.0f32; LANES];
        for j in 0..LANES {
            gains[j] = gain[g * LANES + j];
            biases[j] = bias[g * LANES + j];
        }
        *plan = NormPlan::new(&gains, &biases, &[stream_step; LANES]);
    }
    Stem {
        conv,
        norm: Norm {
            input_step: conv_step,
            plans: norm_plans,
        },
    }
}

/// The stem convolution (4x4, stride 4, 3 -> 24 channels) into `out`, in
/// the `stem.0` mapping: the input widened to eight `i16` channels four
/// rows at a time (`band`).
fn run_stem(stem: &Stem<'_>, input: &[i8], out: &mut [i16], band: &mut [i16]) -> Shape {
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
            &stem.conv,
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

/// The downsample `prefix` from `channels` values of step `stream_step`
/// into the next stage's stream mapping; `wide` as for [`lane_weight`].
fn compile_downsample<'m>(
    weights: &'m impl Weights,
    prefix: &str,
    channels: usize,
    stream_step: f32,
    next_step: f32,
    wide: bool,
    storage: &mut ModelStorage<'m>,
) -> Downsample<'m> {
    let norm_step = step_of(weights, prefix, "0");
    let norm = Norm::plain(channels, stream_step, norm_step, storage);
    let conv = packed(weights, prefix, "1");
    let conv = lane_weight(
        &conv,
        norm_step,
        next_step,
        None,
        4 * channels,
        wide,
        storage,
    );
    Downsample { norm, conv }
}

/// Standardize the stream, then Conv 2x2 stride 2 with the folded
/// weights into the next stage's stream mapping. The result replaces
/// `scratch.x`.
fn run_downsample(downsample: &Downsample<'_>, shape: Shape, scratch: &mut Scratch<'_>) -> Shape {
    let normed = &mut scratch.a[..shape.len()];
    downsample
        .norm
        .run(&scratch.x[..shape.len()], shape.channels, normed);
    lanes::conv2d(
        normed,
        shape,
        &downsample.conv,
        2,
        2,
        0,
        Store::Write,
        scratch.x,
    )
}

/// The ConvBlock `prefix` of `channels` channels on a stream of step
/// `stream_step`; `wide` as for [`lane_weight`].
fn compile_conv_block<'m>(
    weights: &'m impl Weights,
    prefix: &str,
    kernel: usize,
    channels: usize,
    stream_step: f32,
    wide: bool,
    storage: &mut ModelStorage<'m>,
) -> ConvBlock<'m> {
    // The depthwise convolution reads the stream and writes its own
    // mapping.
    let dw_step = step_of(weights, prefix, "conv_dw");
    let dw = Name::join(prefix, "conv_dw");
    let depthwise = weights.get_i8(Name::join(dw.as_str(), "weight").as_str());
    let depthwise_plans = storage.take_plans(channels / LANES);
    plan_channels(
        weights.get(Name::join(dw.as_str(), "scales").as_str()),
        weights.get(Name::join(dw.as_str(), "bias").as_str()),
        &[stream_step],
        dw_step,
        kernel * kernel,
        depthwise_plans,
    );
    let norm_step = step_of(weights, prefix, "norm");
    let norm = Norm::plain(channels, dw_step, norm_step, storage);
    let mlp = compile_mlp(weights, prefix, norm_step, stream_step, wide, storage);
    ConvBlock {
        depthwise,
        depthwise_plans,
        kernel,
        norm,
        mlp,
    }
}

/// `x += gamma * fc2(gelu(fc1(norm(depthwise(x)))))`.
fn run_conv_block(
    block: &ConvBlock<'_>,
    gelu: &GeluTable<'_>,
    shape: Shape,
    scratch: &mut Scratch<'_>,
) {
    let len = shape.len();
    // The depthwise convolution reads the stream and writes `a` in its
    // own mapping.
    lanes::depthwise(
        &scratch.x[..len],
        shape,
        block.depthwise,
        block.depthwise_plans,
        block.kernel,
        1,
        block.kernel / 2,
        Store::Write,
        &mut scratch.a[..len],
    );
    // LayerNorm into `b`, then the MLP into the stream.
    block
        .norm
        .run(&scratch.a[..len], shape.channels, &mut scratch.b[..len]);
    run_mlp(
        &block.mlp,
        gelu,
        &scratch.b[..len],
        &mut scratch.x[..len],
        scratch.hidden,
    );
}

/// The MLP of the block `prefix`: its input is standardized with step
/// `normed_step`, its result is added to a tensor of step `stream_step`;
/// `wide` as for [`lane_weight`].
fn compile_mlp<'m>(
    weights: &'m impl Weights,
    prefix: &str,
    normed_step: f32,
    stream_step: f32,
    wide: bool,
    storage: &mut ModelStorage<'m>,
) -> Mlp<'m> {
    let fc1 = packed(weights, prefix, "mlp.fc1");
    let fc2 = packed(weights, prefix, "mlp.fc2");
    let channels = fc2.bias.len();
    let widened = fc1.bias.len();
    let gamma = weight(weights, prefix, "gamma");
    let fc1 = lane_weight(&fc1, normed_step, GELU_STEP, None, channels, wide, storage);
    let fc2_range = step_of(weights, prefix, "mlp.fc2") * 32767.0;
    let fc2 = lane_weight(
        &fc2,
        GELU_STEP,
        stream_step,
        Some((gamma, fc2_range)),
        widened,
        wide,
        storage,
    );
    Mlp {
        fc1,
        fc2,
        channels,
        widened,
    }
}

/// `stream += gamma * fc2(gelu(fc1(normed)))`: `normed` is the
/// standardized input of the block, `stream` the tensor the result is
/// added to. Each row is its own: when `hidden` cannot hold the whole
/// hidden tensor, strips of rows of about equal size go through both
/// layers one after the other.
fn run_mlp(
    mlp: &Mlp<'_>,
    gelu: &GeluTable<'_>,
    normed: &[i16],
    stream: &mut [i16],
    hidden: &mut [i16],
) {
    let rows = normed.len() / mlp.channels;
    let strips = rows.div_ceil((hidden.len() / mlp.widened).max(1));
    let strip_rows = rows.div_ceil(strips.max(1));
    for (normed, stream) in normed
        .chunks(strip_rows * mlp.channels)
        .zip(stream.chunks_mut(strip_rows * mlp.channels))
    {
        let hidden = &mut hidden[..normed.len() / mlp.channels * mlp.widened];
        lanes::linear(normed, mlp.channels, &mlp.fc1, Store::Gelu(gelu), hidden);
        lanes::linear(hidden, mlp.widened, &mlp.fc2, Store::Add, stream);
    }
}

/// The SplitTransposeBlock `prefix` on a stream of `shape` and step
/// `stream_step`, `wide` as for [`lane_weight`]; see the `f32` version
/// for the structure.
#[allow(clippy::too_many_arguments)]
fn compile_attention_block<'m>(
    weights: &'m impl Weights,
    prefix: &str,
    convs: usize,
    positional: bool,
    shape: Shape,
    stream_step: f32,
    wide: bool,
    storage: &mut ModelStorage<'m>,
) -> AttentionBlock<'m> {
    const CONV_NAMES: [&str; MAX_SPLIT_CONVS] = ["0", "1", "2"];
    let channels = shape.channels;
    let chunk = channels.div_ceil(convs + 1);
    let padded = chunk.div_ceil(LANES) * LANES;
    let tokens_step = step_of(weights, prefix, "tokens");

    // The chain, in one mapping wide enough for every sum and
    // convolution output in it, padded to a multiple of eight channels.
    let mut chain_step = stream_step;
    for name in &CONV_NAMES[..convs] {
        let conv = Name::join("convs", name);
        let full = Name::join(prefix, conv.as_str());
        chain_step = chain_step
            .max(step_of(weights, full.as_str(), "input"))
            .max(step(weights, full.as_str()));
    }
    let split_convs = core::array::from_fn(|index| {
        (index < convs).then(|| {
            // The convolution, weights padded to the chunk.
            let conv = Name::join("convs", CONV_NAMES[index]);
            let full = Name::join(prefix, conv.as_str());
            let dw_weight = weights.get_i8(Name::join(full.as_str(), "weight").as_str());
            let dw_scales = weights.get(Name::join(full.as_str(), "scales").as_str());
            let dw_bias = weights.get(Name::join(full.as_str(), "bias").as_str());
            let padded_weight = storage.take_weights(SPLIT_TAPS * padded);
            padded_weight.fill(0);
            let mut padded_scales = [1.0f32; 48];
            let mut padded_bias = [0.0f32; 48];
            for tap in 0..SPLIT_TAPS {
                for c in 0..chunk {
                    padded_weight[tap * padded + c] = dw_weight[tap * chunk + c];
                }
            }
            padded_scales[..chunk].copy_from_slice(&dw_scales[..chunk]);
            padded_bias[..chunk].copy_from_slice(&dw_bias[..chunk]);
            let plans = storage.take_plans(padded / LANES);
            plan_channels(
                &padded_scales[..padded],
                &padded_bias[..padded],
                &[chain_step],
                chain_step,
                SPLIT_TAPS,
                plans,
            );
            SplitConv {
                weights: padded_weight,
                plans,
            }
        })
    });

    let positional = positional.then(|| {
        // The positional constant, quantized into the tokens' mapping.
        let constant = weight(weights, prefix, "pos_embd.constant");
        let quantized = storage.take_constants(shape.len());
        let inverse = 1.0 / tokens_step;
        for (q, &value) in quantized.iter_mut().zip(constant) {
            *q = Quant {
                scale: tokens_step,
                zero_point: 0,
            }
            .quantize16_with(inverse, value);
        }
        &*quantized
    });

    // Attention: standardize, qkv, the attention itself, the projection
    // added to the tokens.
    let norm_step = step_of(weights, prefix, "norm_xca");
    let norm_xca = Norm::plain(channels, tokens_step, norm_step, storage);
    let qkv_step = step_of(weights, prefix, "xca.qkv");
    let qkv = packed(weights, prefix, "xca.qkv");
    let qkv = lane_weight(&qkv, norm_step, qkv_step, None, channels, wide, storage);
    let mixed_step = step_of(weights, prefix, "xca.mixed");
    let temperature = weight(weights, prefix, "xca.temperature");
    let proj = packed(weights, prefix, "xca.proj");
    let proj_range = step_of(weights, prefix, "xca.proj") * 32767.0;
    let proj = lane_weight(
        &proj,
        mixed_step,
        tokens_step,
        Some((weight(weights, prefix, "gamma_xca"), proj_range)),
        channels,
        wide,
        storage,
    );

    // The MLP on the tokens, added to the block's input.
    let mlp_norm_step = step_of(weights, prefix, "norm");
    let norm = Norm::plain(channels, tokens_step, mlp_norm_step, storage);
    let mlp = compile_mlp(weights, prefix, mlp_norm_step, stream_step, wide, storage);

    AttentionBlock {
        convs: split_convs,
        chunk,
        padded,
        chain_step,
        tokens_step,
        to_chain: lanes::rescale_plan(stream_step, chain_step),
        to_tokens: lanes::rescale_plan(chain_step, tokens_step),
        stream_to_tokens: lanes::rescale_plan(stream_step, tokens_step),
        positional,
        norm_xca,
        qkv,
        qkv_step,
        mixed_step,
        temperature,
        proj,
        norm,
        mlp,
    }
}

/// The split-convolution chain, the attention and the MLP of a
/// SplitTransposeBlock; see the `f32` version for the structure.
#[allow(clippy::too_many_arguments)]
fn run_attention_block(
    block: &AttentionBlock<'_>,
    gelu: &GeluTable<'_>,
    prefix: &str,
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
    let (chunk, padded) = (block.chunk, block.padded);
    let chunk_shape = Shape::new(shape.height, shape.width, padded);
    let mut adds = 0;
    let add_name = |adds: usize| {
        const NAMES: [&str; 5] = ["Add", "Add_1", "Add_2", "Add_3", "Add_4"];
        NAMES[adds]
    };

    let (mut running, mut filtered) = (&mut *scratch.chain.0, &mut *scratch.chain.1);
    let (to_chain, to_chain_shift) = block.to_chain;
    let (to_tokens, to_tokens_shift) = block.to_tokens;
    let (stream_to_tokens, stream_to_tokens_shift) = block.stream_to_tokens;
    let x = &scratch.x[..len];
    let tokens = &mut scratch.tokens[..len];
    let mut convs = 0;
    for (index, conv) in block.convs.iter().flatten().enumerate() {
        convs += 1;
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
                block.chain_step,
            );
        }
        lanes::depthwise(
            &running[..pixels * padded],
            chunk_shape,
            conv.weights,
            conv.plans,
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

    if let Some(positional) = block.positional {
        lanes::add(tokens, positional);
        adds += 1;
        emit(
            trace,
            trace_buffer,
            Name::join(prefix, add_name(adds)).as_str(),
            shape,
            tokens,
            block.tokens_step,
        );
    }

    // Attention: standardize into `a`, qkv, the attention itself, the
    // projection added to the tokens.
    let normed = &mut scratch.a[..len];
    block.norm_xca.run(tokens, channels, normed);
    let packed_width = 3 * channels;
    let qkv = &mut scratch.qkv[..pixels * packed_width];
    lanes::linear(normed, channels, &block.qkv, Store::Write, qkv);
    let mixed = &mut scratch.b[..len];
    cross_covariance_attention(
        block.temperature,
        shape,
        qkv,
        block.qkv_step,
        scratch.heads,
        scratch.mixed,
        attention,
        block.mixed_step,
        mixed,
    );
    lanes::linear(mixed, channels, &block.proj, Store::Add, tokens);
    adds += 1;
    emit(
        trace,
        trace_buffer,
        Name::join(prefix, add_name(adds)).as_str(),
        shape,
        tokens,
        block.tokens_step,
    );

    // The MLP on the tokens, added to the block's input.
    let normed = &mut scratch.a[..len];
    block.norm.run(tokens, channels, normed);
    run_mlp(
        &block.mlp,
        gelu,
        normed,
        &mut scratch.x[..len],
        scratch.hidden,
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

/// The head on `channels` pooled values of step `stream_step`.
fn compile_head<'m>(
    weights: &'m impl Weights,
    channels: usize,
    stream_step: f32,
    storage: &mut ModelStorage<'m>,
) -> Head<'m> {
    let norm_step = step(weights, "head.norm");
    let norm = Norm::plain(channels, stream_step, norm_step, storage);
    let embedding_step = step(weights, "embedding");
    let fc = packed(weights, "head", "fc");
    let fc = lane_weight(
        &fc,
        norm_step,
        embedding_step,
        None,
        channels,
        false,
        storage,
    );
    Head {
        norm,
        fc,
        embedding_step,
    }
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
