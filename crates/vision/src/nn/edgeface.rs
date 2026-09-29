//! EdgeFace-XXS, the face recognizer: an aligned 112x112 face in, 512
//! numbers out.
//!
//! The network is an EdgeNeXt-XXS backbone (Maaz et al., 2022) trained for
//! faces (George et al., "EdgeFace", 2023). This is the `f32` reference,
//! written to follow the PyTorch source block by block. In PyTorch terms:
//!
//! ```text
//! stem:     Conv 4x4 stride 4 (3 -> 24), LayerNorm
//! stage 0:  2 x ConvBlock(24, kernel 3)
//! stage 1:  Downsample(24 -> 48), ConvBlock(48, kernel 5),
//!           SplitTransposeBlock(48, 1 split conv, positional encoding)
//! stage 2:  Downsample(48 -> 88), 5 x ConvBlock(88, kernel 7),
//!           SplitTransposeBlock(88, 2 split convs)
//! stage 3:  Downsample(88 -> 168), ConvBlock(168, kernel 9),
//!           SplitTransposeBlock(168, 3 split convs)
//! head:     global average, LayerNorm, Linear (168 -> 512)
//! ```
//!
//! - `ConvBlock(C, k)`: `x + gamma * fc2(gelu(fc1(norm(depthwise_k(x)))))`,
//!   where `fc1` widens to `4C` and `fc2` narrows back.
//! - `Downsample`: LayerNorm, then Conv 2x2 stride 2.
//! - `SplitTransposeBlock(C, n)`: split the channels into `n + 1` chunks;
//!   pass a running sum of the first `n` through depthwise 3x3
//!   convolutions; then, on the pixels as tokens, add the positional
//!   encoding (stage 1 only), add `gamma_xca * xca(norm_xca(x))`, and end
//!   like a ConvBlock without the depthwise convolution:
//!   `input + gamma * fc2(gelu(fc1(norm(x))))`.
//! - `xca` (cross-covariance attention): per head, the channels attend to
//!   each other with a `C/4 x C/4` matrix computed from L2-normalized
//!   queries and keys over the pixels.
//!
//! Weight names are the PyTorch parameter names without `model.`, as
//! `tools/facekit` exports them (see `tests/fixtures/edgeface_xxs.f32.txt`).
//! The output is the raw `Linear` result: it is not normalized. Callers
//! L2-normalize it before comparing embeddings.
//!
//! All arithmetic is channels-last. The pixels-as-tokens layout of the
//! attention block, `[pixel][channel]`, is the same memory as
//! `[row][column][channel]`, so the block needs no transposes.
//!
//! [`forward_probed`] also reports every tensor that the integer version
//! ([`int8`]) stores as `i8`, so that `facekit quantize` can measure their
//! ranges on a calibration set. The names of those probes are the names
//! the integer version reads its `q.<name>` mappings by; both are listed
//! in the [`int8`] docs.

pub mod int8;

use super::{
    Activation, Shape, Weights, add, add_scaled, affine_rows, conv2d, depthwise, gelu,
    global_average, layer_norm, linear, quant::Granularity, softmax_rows, standardize_rows,
};

/// Rows and columns of the input face.
pub const INPUT_SIZE: usize = 112;
/// Values in the output embedding.
pub const EMBEDDING_LEN: usize = 512;
/// Epsilon of every LayerNorm in the model.
pub(super) const LAYER_NORM_EPSILON: f32 = 1e-6;
/// Epsilon of the L2 normalization inside the attention (PyTorch's
/// `F.normalize` default).
pub(super) const NORMALIZE_EPSILON: f32 = 1e-12;
/// Attention heads in every SplitTransposeBlock.
pub(super) const HEADS: usize = 4;
/// The widest activation tensor: after the stem, 28x28x24.
pub(super) const MAX_ACTIVATION: usize = 28 * 28 * 24;
/// The widest MLP hidden tensor: after `fc1` of stage 0, 28x28x96.
const MAX_HIDDEN: usize = 28 * 28 * 96;
/// The largest attention matrix: 4 heads of 42x42 in stage 3.
pub(super) const MAX_ATTENTION: usize = HEADS * 42 * 42;
/// The channel counts of the four stages.
pub(super) const STAGE_CHANNELS: [usize; 4] = [24, 48, 88, 168];
/// The depthwise kernel size of the ConvBlocks of each stage.
pub(super) const STAGE_KERNELS: [usize; 4] = [3, 5, 7, 9];
/// The number of ConvBlocks in each stage before its SplitTransposeBlock
/// (stage 0 has no SplitTransposeBlock).
pub(super) const STAGE_CONV_BLOCKS: [usize; 4] = [2, 1, 5, 1];
/// The number of split convolutions in the SplitTransposeBlock of stages
/// 1 to 3.
pub(super) const STAGE_SPLIT_CONVS: [usize; 3] = [1, 2, 3];

/// The scratch memory one forward pass needs, in `f32` values. The caller
/// allocates a slice of this length once; [`Scratch::new`] splits it.
pub const SCRATCH_LEN: usize = 4 * MAX_ACTIVATION + MAX_HIDDEN + MAX_ATTENTION;

/// The working buffers of a forward pass, borrowed from one caller slice.
pub struct Scratch<'a> {
    /// The residual stream: the tensor that flows from block to block.
    x: &'a mut [f32],
    /// A block's working copy of the stream, and the split-conv chain.
    t1: &'a mut [f32],
    /// The tokens of a SplitTransposeBlock, and downsample outputs.
    t2: &'a mut [f32],
    /// The output of an attention or MLP branch before it joins the stream.
    t3: &'a mut [f32],
    /// The MLP hidden tensor, and the packed queries, keys and values.
    hidden: &'a mut [f32],
    /// The attention matrices of all heads.
    attention: &'a mut [f32],
}

impl<'a> Scratch<'a> {
    /// Split `memory` into the working buffers.
    ///
    /// # Panics
    ///
    /// When `memory` is shorter than [`SCRATCH_LEN`].
    pub fn new(memory: &'a mut [f32]) -> Self {
        assert!(memory.len() >= SCRATCH_LEN, "EdgeFace scratch memory");
        let (x, rest) = memory.split_at_mut(MAX_ACTIVATION);
        let (t1, rest) = rest.split_at_mut(MAX_ACTIVATION);
        let (t2, rest) = rest.split_at_mut(MAX_ACTIVATION);
        let (t3, rest) = rest.split_at_mut(MAX_ACTIVATION);
        let (hidden, rest) = rest.split_at_mut(MAX_HIDDEN);
        let (attention, _) = rest.split_at_mut(MAX_ATTENTION);
        Self {
            x,
            t1,
            t2,
            t3,
            hidden,
            attention,
        }
    }
}

/// A weight name built from a block prefix and a parameter name, without
/// allocating: `stages.2.blocks.5.xca.qkv.weight` and the like.
pub(super) struct Name {
    /// The bytes of the name.
    pub(super) bytes: [u8; 64],
    /// How many of them are used.
    pub(super) len: usize,
}

impl Name {
    /// `prefix.suffix`.
    ///
    /// # Panics
    ///
    /// When the name does not fit in 64 bytes (no exported name is longer).
    pub(super) fn join(prefix: &str, suffix: &str) -> Self {
        let mut bytes = [0u8; 64];
        let len = prefix.len() + 1 + suffix.len();
        assert!(len <= bytes.len(), "weight name too long");
        bytes[..prefix.len()].copy_from_slice(prefix.as_bytes());
        bytes[prefix.len()] = b'.';
        bytes[prefix.len() + 1..len].copy_from_slice(suffix.as_bytes());
        Self { bytes, len }
    }

    /// The name as text.
    pub(super) fn as_str(&self) -> &str {
        // The bytes came from two `&str`s and a dot, so they are UTF-8.
        core::str::from_utf8(&self.bytes[..self.len]).unwrap_or("")
    }
}

/// The weight `prefix.suffix`.
pub(super) fn weight<'w>(weights: &'w impl Weights, prefix: &str, suffix: &str) -> &'w [f32] {
    weights.get(Name::join(prefix, suffix).as_str())
}

/// A block name: `stages.<stage>.blocks.<block>`, or the stage's
/// `stages.<stage>.downsample`.
pub(super) struct BlockName {
    /// The bytes of the name.
    bytes: [u8; 32],
    /// How many of them are used.
    len: usize,
}

impl BlockName {
    /// `stages.<stage>.blocks.<block>`; `block` is at most 9.
    pub(super) fn block(stage: usize, block: usize) -> Self {
        let mut name = Self::stage(stage, ".blocks.");
        name.push_digit(block);
        name
    }

    /// `stages.<stage>.downsample`.
    pub(super) fn downsample(stage: usize) -> Self {
        Self::stage(stage, ".downsample")
    }

    /// `stages.<stage>.stream`: the residual stream inside the stage.
    pub(super) fn stream(stage: usize) -> Self {
        Self::stage(stage, ".stream")
    }

    /// `stages.<stage><suffix>`; `stage` is at most 9.
    fn stage(stage: usize, suffix: &str) -> Self {
        let mut name = Self {
            bytes: [0; 32],
            len: 0,
        };
        name.push_str("stages.");
        name.push_digit(stage);
        name.push_str(suffix);
        name
    }

    /// Append text.
    fn push_str(&mut self, text: &str) {
        self.bytes[self.len..self.len + text.len()].copy_from_slice(text.as_bytes());
        self.len += text.len();
    }

    /// Append one decimal digit.
    fn push_digit(&mut self, digit: usize) {
        assert!(digit < 10, "block index {digit} needs two digits");
        self.bytes[self.len] = b'0' + digit as u8;
        self.len += 1;
    }

    /// The name as text.
    pub(super) fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.len]).unwrap_or("")
    }
}

/// Run the recognizer on `input`: 112x112 pixels, 3 channels (R, G, B),
/// each value `(byte / 255 - 0.5) / 0.5`. Write the 512 raw embedding
/// values to `embedding`.
///
/// # Panics
///
/// When the input is not 112x112x3, `embedding` is shorter than 512, or
/// `scratch` is too small.
pub fn forward(weights: &impl Weights, input: &[f32], scratch: &mut [f32], embedding: &mut [f32]) {
    forward_probed(
        weights,
        input,
        scratch,
        embedding,
        |_, _, _| {},
        |_, _, _| {},
    );
}

/// [`forward`], calling `trace(name, shape, values)` after every block with
/// the block's output, under the names of the golden files
/// (`stages.2.blocks.5.Add_3` is the final sum of that block, and so on).
/// For the token-shaped tensors of the attention blocks, `shape` has the
/// pixel grid of the stage; the values are the same memory either way.
pub fn forward_traced(
    weights: &impl Weights,
    input: &[f32],
    scratch: &mut [f32],
    embedding: &mut [f32],
    trace: impl FnMut(&str, Shape, &[f32]),
) {
    forward_probed(weights, input, scratch, embedding, trace, |_, _, _| {});
}

/// [`forward_traced`], additionally calling `probe(name, granularity,
/// values)` with every tensor that the integer version quantizes, under
/// the name it reads the mapping by and with the granularity it wants.
/// `facekit quantize` collects the ranges of these. A LayerNorm output is
/// reported standardized, before its scale and shift: the integer version
/// folds those into the next layer's weights.
pub fn forward_probed(
    weights: &impl Weights,
    input: &[f32],
    scratch: &mut [f32],
    embedding: &mut [f32],
    mut trace: impl FnMut(&str, Shape, &[f32]),
    mut probe: impl FnMut(&str, Granularity, &[f32]),
) {
    assert_eq!(input.len(), INPUT_SIZE * INPUT_SIZE * 3, "EdgeFace input");
    assert!(
        embedding.len() >= EMBEDDING_LEN,
        "EdgeFace embedding buffer"
    );
    let mut scratch = Scratch::new(scratch);

    // Stem: Conv 4x4 stride 4, then LayerNorm over the 24 channels.
    let mut shape = conv2d(
        input,
        Shape::new(INPUT_SIZE, INPUT_SIZE, 3),
        weights.get("stem.0.weight"),
        weights.get("stem.0.bias"),
        4,
        4,
        0,
        Activation::None,
        scratch.x,
    );
    let x = &mut scratch.x[..shape.len()];
    probe("stem.0", Granularity::Wide, x);
    layer_norm(
        x,
        shape.channels,
        weights.get("stem.1.weight"),
        weights.get("stem.1.bias"),
        LAYER_NORM_EPSILON,
    );
    trace("stem.1.Transpose_1", shape, x);

    for stage in 0..4 {
        if stage > 0 {
            let name = BlockName::downsample(stage);
            shape = downsample(weights, name.as_str(), shape, &mut scratch, &mut probe);
            trace(
                Name::join(name.as_str(), "1.Conv").as_str(),
                shape,
                &scratch.x[..shape.len()],
            );
        }
        // The residual stream everywhere in the stage, for its 16-bit
        // mapping: at the input of every block and at the end.
        let stream_name = BlockName::stream(stage);
        probe(
            stream_name.as_str(),
            Granularity::Wide,
            &scratch.x[..shape.len()],
        );
        for block in 0..STAGE_CONV_BLOCKS[stage] {
            let name = BlockName::block(stage, block);
            conv_block(
                weights,
                name.as_str(),
                STAGE_KERNELS[stage],
                shape,
                &mut scratch,
                &mut probe,
            );
            trace(
                Name::join(name.as_str(), "Add").as_str(),
                shape,
                &scratch.x[..shape.len()],
            );
            probe(
                stream_name.as_str(),
                Granularity::Wide,
                &scratch.x[..shape.len()],
            );
        }
        if stage > 0 {
            let name = BlockName::block(stage, STAGE_CONV_BLOCKS[stage]);
            split_transpose_block(
                weights,
                name.as_str(),
                STAGE_SPLIT_CONVS[stage - 1],
                stage == 1,
                shape,
                &mut scratch,
                &mut trace,
                &mut probe,
            );
            probe(
                stream_name.as_str(),
                Granularity::Wide,
                &scratch.x[..shape.len()],
            );
        }
        debug_assert_eq!(shape.channels, STAGE_CHANNELS[stage]);
    }

    // Head: mean over the 3x3 pixels, LayerNorm, Linear.
    let channels = shape.channels;
    let pooled = &mut scratch.t1[..channels];
    global_average(&scratch.x[..shape.len()], shape, pooled);
    trace(
        "head.global_pool.pool.GlobalAveragePool",
        Shape::new(1, 1, channels),
        pooled,
    );
    standardize_rows(pooled, channels, LAYER_NORM_EPSILON);
    probe("head.norm", Granularity::Wide, pooled);
    affine_rows(
        pooled,
        weights.get("head.norm.weight"),
        weights.get("head.norm.bias"),
    );
    trace(
        "head.norm.LayerNormalization",
        Shape::new(1, 1, channels),
        pooled,
    );
    linear(
        pooled,
        channels,
        weights.get("head.fc.weight"),
        Some(weights.get("head.fc.bias")),
        EMBEDDING_LEN,
        Activation::None,
        embedding,
    );
    probe("embedding", Granularity::Wide, &embedding[..EMBEDDING_LEN]);
    trace(
        "embedding",
        Shape::new(1, 1, EMBEDDING_LEN),
        &embedding[..EMBEDDING_LEN],
    );
}

/// LayerNorm, then Conv 2x2 stride 2 into the next stage's channel count.
/// The result replaces the stream in `scratch.x`; returns its shape.
fn downsample(
    weights: &impl Weights,
    prefix: &str,
    shape: Shape,
    scratch: &mut Scratch<'_>,
    probe: &mut impl FnMut(&str, Granularity, &[f32]),
) -> Shape {
    let x = &scratch.x[..shape.len()];
    let normed = &mut scratch.t1[..shape.len()];
    normed.copy_from_slice(x);
    standardize_rows(normed, shape.channels, LAYER_NORM_EPSILON);
    probe(Name::join(prefix, "0").as_str(), Granularity::Wide, normed);
    affine_rows(
        normed,
        weight(weights, prefix, "0.weight"),
        weight(weights, prefix, "0.bias"),
    );
    let out = conv2d(
        normed,
        shape,
        weight(weights, prefix, "1.weight"),
        weight(weights, prefix, "1.bias"),
        2,
        2,
        0,
        Activation::None,
        scratch.t2,
    );
    scratch.x[..out.len()].copy_from_slice(&scratch.t2[..out.len()]);
    out
}

/// `x += gamma * fc2(gelu(fc1(norm(depthwise(x)))))`, in `scratch.x`.
fn conv_block(
    weights: &impl Weights,
    prefix: &str,
    kernel: usize,
    shape: Shape,
    scratch: &mut Scratch<'_>,
    probe: &mut impl FnMut(&str, Granularity, &[f32]),
) {
    let x = &mut scratch.x[..shape.len()];
    let branch = &mut scratch.t1[..shape.len()];
    probe(
        Name::join(prefix, "conv_dw.input").as_str(),
        Granularity::WideChannel(shape.channels),
        x,
    );
    depthwise(
        x,
        shape,
        weight(weights, prefix, "conv_dw.weight"),
        weight(weights, prefix, "conv_dw.bias"),
        kernel,
        1,
        kernel / 2,
        Activation::None,
        branch,
    );
    probe(
        Name::join(prefix, "conv_dw").as_str(),
        Granularity::Wide,
        branch,
    );
    mlp_branch(
        weights,
        prefix,
        shape,
        branch,
        scratch.hidden,
        &mut scratch.t3[..shape.len()],
        probe,
    );
    add_scaled(
        x,
        &scratch.t3[..shape.len()],
        weight(weights, prefix, "gamma"),
    );
}

/// `output = fc2(gelu(fc1(norm(input))))` for every pixel; `input` is
/// normalized in place. `hidden` holds the widened tensor.
fn mlp_branch(
    weights: &impl Weights,
    prefix: &str,
    shape: Shape,
    input: &mut [f32],
    hidden: &mut [f32],
    output: &mut [f32],
    probe: &mut impl FnMut(&str, Granularity, &[f32]),
) {
    let channels = shape.channels;
    standardize_rows(input, channels, LAYER_NORM_EPSILON);
    probe(
        Name::join(prefix, "norm").as_str(),
        Granularity::Wide,
        input,
    );
    affine_rows(
        input,
        weight(weights, prefix, "norm.weight"),
        weight(weights, prefix, "norm.bias"),
    );
    let widened = 4 * channels;
    let hidden = &mut hidden[..shape.pixels() * widened];
    linear(
        input,
        channels,
        weight(weights, prefix, "mlp.fc1.weight"),
        Some(weight(weights, prefix, "mlp.fc1.bias")),
        widened,
        Activation::None,
        hidden,
    );
    probe(
        Name::join(prefix, "mlp.fc1").as_str(),
        Granularity::Tensor,
        hidden,
    );
    gelu(hidden);
    probe(
        Name::join(prefix, "mlp.act").as_str(),
        Granularity::Wide,
        hidden,
    );
    linear(
        hidden,
        widened,
        weight(weights, prefix, "mlp.fc2.weight"),
        Some(weight(weights, prefix, "mlp.fc2.bias")),
        channels,
        Activation::None,
        output,
    );
    probe(
        Name::join(prefix, "mlp.fc2").as_str(),
        Granularity::Wide,
        output,
    );
}

/// Copy channels `range` of every pixel of `source` (with `channels` per
/// pixel) to `target`, which has `range.len()` channels per pixel.
pub(super) fn gather_channels(
    source: &[f32],
    channels: usize,
    range: core::ops::Range<usize>,
    target: &mut [f32],
) {
    let width = range.len();
    for (pixel, slot) in source
        .chunks_exact(channels)
        .zip(target.chunks_exact_mut(width))
    {
        slot.copy_from_slice(&pixel[range.clone()]);
    }
}

/// Copy `source` (with `range.len()` channels per pixel) into channels
/// `range` of every pixel of `target` (with `channels` per pixel).
pub(super) fn scatter_channels(
    source: &[f32],
    target: &mut [f32],
    channels: usize,
    range: core::ops::Range<usize>,
) {
    let width = range.len();
    for (slot, pixel) in source
        .chunks_exact(width)
        .zip(target.chunks_exact_mut(channels))
    {
        pixel[range.clone()].copy_from_slice(slot);
    }
}

/// The split-convolution chain, the attention and the MLP of a
/// SplitTransposeBlock, in `scratch.x`. `convs` is the number of split
/// convolutions; `positional` says whether the block adds the positional
/// encoding. Calls `trace` with the intermediate sums the golden files
/// record.
// The block has two observers (trace and probe) on top of its inputs.
#[allow(clippy::too_many_arguments)]
fn split_transpose_block(
    weights: &impl Weights,
    prefix: &str,
    convs: usize,
    positional: bool,
    shape: Shape,
    scratch: &mut Scratch<'_>,
    trace: &mut impl FnMut(&str, Shape, &[f32]),
    probe: &mut impl FnMut(&str, Granularity, &[f32]),
) {
    let channels = shape.channels;
    let pixels = shape.pixels();
    let len = shape.len();
    // PyTorch's `chunk(n + 1)`: chunks of `ceil(C / (n + 1))`, the last one
    // takes the rest.
    let chunk = channels.div_ceil(convs + 1);
    let chunk_shape = Shape::new(shape.height, shape.width, chunk);
    // Golden names of the chain sums are Add_1, Add_2, ...; the attention
    // sum and the final sum continue the count.
    let mut adds = 0;
    let add_name = |adds: usize| {
        const NAMES: [&str; 5] = ["Add", "Add_1", "Add_2", "Add_3", "Add_4"];
        NAMES[adds]
    };

    // The chain: sp = conv_i(sp + chunk_i), concatenated into `tokens`.
    let x = &scratch.x[..len];
    let tokens = &mut scratch.t2[..len];
    let running = &mut scratch.t1[..pixels * chunk];
    let filtered = &mut scratch.t3[..pixels * chunk];
    for index in 0..convs {
        let range = index * chunk..(index + 1) * chunk;
        if index == 0 {
            gather_channels(x, channels, range.clone(), running);
        } else {
            gather_channels(x, channels, range.clone(), filtered);
            add(running, filtered);
            adds += 1;
            trace(
                Name::join(prefix, add_name(adds)).as_str(),
                chunk_shape,
                running,
            );
        }
        let mut conv = Name::join(prefix, "convs.");
        conv.bytes[conv.len] = b'0' + index as u8;
        conv.len += 1;
        probe(
            Name::join(conv.as_str(), "input").as_str(),
            Granularity::Wide,
            running,
        );
        depthwise(
            running,
            chunk_shape,
            weight(weights, conv.as_str(), "weight"),
            weight(weights, conv.as_str(), "bias"),
            3,
            1,
            1,
            Activation::None,
            filtered,
        );
        probe(conv.as_str(), Granularity::Wide, filtered);
        running.copy_from_slice(filtered);
        scatter_channels(running, tokens, channels, range);
    }
    let last = convs * chunk..channels;
    for (pixel, slot) in x
        .chunks_exact(channels)
        .zip(tokens.chunks_exact_mut(channels))
    {
        slot[last.clone()].copy_from_slice(&pixel[last.clone()]);
    }

    // Pixels become tokens: the same memory. Positional encoding, then
    // attention on a normalized copy, scaled into the tokens.
    if positional {
        add(tokens, weight(weights, prefix, "pos_embd.constant"));
        adds += 1;
        trace(Name::join(prefix, add_name(adds)).as_str(), shape, tokens);
    }
    probe(
        Name::join(prefix, "tokens").as_str(),
        Granularity::Wide,
        tokens,
    );
    let normed = &mut scratch.t1[..len];
    normed.copy_from_slice(tokens);
    standardize_rows(normed, channels, LAYER_NORM_EPSILON);
    probe(
        Name::join(prefix, "norm_xca").as_str(),
        Granularity::Wide,
        normed,
    );
    affine_rows(
        normed,
        weight(weights, prefix, "norm_xca.weight"),
        weight(weights, prefix, "norm_xca.bias"),
    );
    let attended = &mut scratch.t3[..len];
    cross_covariance_attention(
        weights,
        prefix,
        shape,
        normed,
        scratch.hidden,
        scratch.attention,
        attended,
        probe,
    );
    add_scaled(tokens, attended, weight(weights, prefix, "gamma_xca"));
    adds += 1;
    trace(Name::join(prefix, add_name(adds)).as_str(), shape, tokens);
    probe(
        Name::join(prefix, "tokens").as_str(),
        Granularity::Wide,
        tokens,
    );

    // The MLP on the tokens, added to the block's input.
    let normed = &mut scratch.t1[..len];
    normed.copy_from_slice(tokens);
    let branch = &mut scratch.t3[..len];
    mlp_branch(
        weights,
        prefix,
        shape,
        normed,
        scratch.hidden,
        branch,
        probe,
    );
    add_scaled(
        &mut scratch.x[..len],
        branch,
        weight(weights, prefix, "gamma"),
    );
    adds += 1;
    trace(
        Name::join(prefix, add_name(adds)).as_str(),
        shape,
        &scratch.x[..len],
    );
}

/// Cross-covariance attention (XCA) over the pixels of `input` as tokens
/// (`shape.pixels()` of them, `shape.channels` values each), written to
/// `output`.
///
/// Per head of `channels / 4` channels: queries and keys are the head's
/// channels as vectors over the tokens, L2-normalized; the attention
/// matrix is `softmax(temperature * q k^T)` (channels by channels); the
/// output channel `c` is the attention-weighted sum of the value
/// channels. Then a linear projection over all channels.
#[allow(clippy::too_many_arguments)]
fn cross_covariance_attention(
    weights: &impl Weights,
    prefix: &str,
    shape: Shape,
    input: &[f32],
    qkv: &mut [f32],
    attention: &mut [f32],
    output: &mut [f32],
    probe: &mut impl FnMut(&str, Granularity, &[f32]),
) {
    let (pixels, channels) = (shape.pixels(), shape.channels);
    let per_head = channels / HEADS;
    let packed = 3 * channels;
    // qkv[token][s * C + h * per_head + c] for s = query, key, value.
    let qkv = &mut qkv[..pixels * packed];
    linear(
        input,
        channels,
        weight(weights, prefix, "xca.qkv.weight"),
        Some(weight(weights, prefix, "xca.qkv.bias")),
        packed,
        Activation::None,
        qkv,
    );
    probe(
        Name::join(prefix, "xca.qkv").as_str(),
        Granularity::Wide,
        qkv,
    );
    let temperature = weight(weights, prefix, "xca.temperature");
    // `mixed` holds, per token, the attention output for every channel; it
    // is the input of the projection.
    let mixed = &mut output[..pixels * channels];

    for head in 0..HEADS {
        let attention =
            &mut attention[head * per_head * per_head..(head + 1) * per_head * per_head];
        let query = head * per_head;
        let key = channels + head * per_head;
        let value = 2 * channels + head * per_head;

        // Normalize each query and key channel over the tokens.
        for column in (query..query + per_head).chain(key..key + per_head) {
            let mut norm = 0.0f32;
            for token in 0..pixels {
                let v = qkv[token * packed + column];
                norm += v * v;
            }
            let scale = 1.0 / libm::sqrtf(norm).max(NORMALIZE_EPSILON);
            for token in 0..pixels {
                qkv[token * packed + column] *= scale;
            }
        }

        // attention[c][c'] = temperature * sum over tokens of q[c] k[c'].
        for c in 0..per_head {
            for c2 in 0..per_head {
                let mut sum = 0.0f32;
                for token in 0..pixels {
                    sum += qkv[token * packed + query + c] * qkv[token * packed + key + c2];
                }
                attention[c * per_head + c2] = sum * temperature[head];
            }
        }
        softmax_rows(attention, per_head);

        // mixed[token][h * per_head + c] = sum over c' of attention[c][c'] v[c'][token].
        for token in 0..pixels {
            let values = &qkv[token * packed + value..token * packed + value + per_head];
            for c in 0..per_head {
                let row = &attention[c * per_head..(c + 1) * per_head];
                mixed[token * channels + head * per_head + c] =
                    row.iter().zip(values).map(|(a, v)| a * v).sum();
            }
        }
    }
    probe(
        Name::join(prefix, "xca.mixed").as_str(),
        Granularity::Wide,
        mixed,
    );

    // Projection, in place through the qkv buffer as temporary storage.
    let projected = &mut qkv[..pixels * channels];
    linear(
        mixed,
        channels,
        weight(weights, prefix, "xca.proj.weight"),
        Some(weight(weights, prefix, "xca.proj.bias")),
        channels,
        Activation::None,
        projected,
    );
    probe(
        Name::join(prefix, "xca.proj").as_str(),
        Granularity::Wide,
        projected,
    );
    mixed.copy_from_slice(projected);
}
