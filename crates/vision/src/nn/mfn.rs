//! MFN_S8_V1, Espressif's face recognizer: a MobileFaceNet whose every
//! tensor is `i8` with one power-of-two scale, published by Espressif
//! under the MIT license (`esp-dl/models/human_face_recognition`).
//!
//! It takes the aligned 112x112 RGB crop of [`align`](crate::align),
//! `(byte - 127.5) / 127.5` quantized with [`INPUT_EXPONENT`], and gives a
//! 512-number embedding. `facekit import-espdl` turns Espressif's
//! `.espdl` file into the FKB1 file this module reads; `facekit eval`
//! measures 99.27 % on LFW with the firmware's alignment.
//!
//! # The network
//!
//! | Part | Layers | Output |
//! | --- | --- | --- |
//! | stem | 3x3 convolution, stride 2, PReLU; depthwise 3x3, PReLU | 56x56x64 |
//! | `dconv_23` | 1x1 to 128, depthwise stride 2, 1x1 to 64 | 28x28x64 |
//! | `res_3_block0..3` | 1x1 to 128, depthwise, 1x1 to 64, residual | 28x28x64 |
//! | `dconv_34` | 1x1 to 256, depthwise stride 2, 1x1 to 128 | 14x14x128 |
//! | `res_4_block0..5` | 1x1 to 256, depthwise, 1x1 to 128, residual | 14x14x128 |
//! | `dconv_45` | 1x1 to 512, depthwise stride 2, 1x1 to 128 | 7x7x128 |
//! | `res_5_block0..1` | 1x1 to 256, depthwise, 1x1 to 128, residual | 7x7x128 |
//! | head | 1x1 to 512, PReLU; depthwise 7x7 over the whole map; 1x1 | 512 |
//!
//! Every 1x1 and depthwise layer but the last two has a PReLU. 221
//! million products per face, 93 percent of them in 1x1 layers.
//!
//! # How it runs
//!
//! On the kernels of [`s8`]. The tensors between blocks are whole, in
//! two buffers that take turns (in PSRAM on the board). Each block runs
//! in bands of rows ([`block::run_banded`]) through three buffers that
//! should be in internal RAM: the larger they are, the taller the bands
//! and the fewer times each layer reads its weights. The stem runs the
//! same way, its 3x3 convolution as a 1x1 over the 27 values around each
//! output pixel (padded to 32). The head's 7x7 depthwise layer has one
//! output pixel and runs in scalar code.
//!
//! The computer runs the same integer arithmetic in scalar code, bit for
//! bit; `facekit` checks it against an interpreter of the `.espdl` graph
//! that shares no code with this module.

use super::s8::{
    self, Depthwise, GroupPrelu, LANES, Plan, Pointwise, Store,
    block::{self, Block},
};
use crate::{align::CROP_SIZE, blob::Blob, image::RgbImage};

/// Values in the embedding.
pub const EMBEDDING_LEN: usize = 512;

/// The power-of-two exponent of the input's scale: a value `q` stands for
/// `q * 2^INPUT_EXPONENT`.
pub const INPUT_EXPONENT: i32 = -6;

/// Plans of all layers together.
pub const MODEL_PLANS: usize = 636;

/// The bytes of each of the two tensors between blocks: the stem's
/// output, 56x56x64.
pub const TENSOR_LEN: usize = 56 * 56 * 64;

/// The least ring a pass works with: four rows of `dconv_23`'s widened
/// tensor (56 pixels of 128 channels), its band of one row at stride 2.
pub const MIN_RING: usize = 4 * 56 * 128;

/// The least band of depthwise rows: one row of `dconv_23`'s output.
pub const MIN_FILTERED: usize = 28 * 128;

/// The least band of output rows: one row of `dconv_23`'s output.
pub const MIN_STAGING: usize = 28 * 64;

/// The ring that lets the 14x14 and 28x28 blocks run in bands of seven
/// rows, the best measured on the board (`mfnbench-4`).
pub const FAST_RING: usize = 9 * 14 * 256;

/// The band of depthwise rows that goes with [`FAST_RING`].
pub const FAST_FILTERED: usize = 7 * 14 * 256;

/// The band of output rows that goes with [`FAST_RING`].
pub const FAST_STAGING: usize = 7 * 14 * 128;

/// The bytes of the stem's columns: one output row of 56 pixels, 32
/// values each.
pub const COLUMNS_LEN: usize = 56 * STEM_TAPS;

/// The stem convolution's values per output pixel: 3x3x3, padded to 32.
const STEM_TAPS: usize = 32;

/// The side of the stem's output.
const STEM_SIDE: usize = 56;

/// The stem's output channels.
const STEM_CHANNELS: usize = 64;

/// The building blocks in order: the name prefix of their three layers,
/// the side of their input, and whether they add their input (stride 1)
/// or halve the image (stride 2).
const BLOCKS: [(&str, usize, bool); 15] = [
    ("dconv_23", 56, false),
    ("res_3_block0", 28, true),
    ("res_3_block1", 28, true),
    ("res_3_block2", 28, true),
    ("res_3_block3", 28, true),
    ("dconv_34", 28, false),
    ("res_4_block0", 14, true),
    ("res_4_block1", 14, true),
    ("res_4_block2", 14, true),
    ("res_4_block3", 14, true),
    ("res_4_block4", 14, true),
    ("res_4_block5", 14, true),
    ("dconv_45", 14, false),
    ("res_5_block0", 7, true),
    ("res_5_block1", 7, true),
];

/// The side of the head's input.
const HEAD_SIDE: usize = 7;

/// The quantized input value of each byte of the crop:
/// `round((byte - 127.5) / 127.5 * 2^-INPUT_EXPONENT)`. No byte falls on
/// a half, so the rounding direction does not matter.
pub const fn input_value(byte: u8) -> i8 {
    let scaled = (2 * byte as i32 - 255) << -INPUT_EXPONENT;
    let rounded = if scaled >= 0 {
        (scaled + 127) / 255
    } else {
        -((-scaled + 127) / 255)
    };
    if rounded > 127 {
        127
    } else if rounded < -128 {
        -128
    } else {
        rounded as i8
    }
}

/// The recognizer's input from an aligned crop: each byte through
/// [`input_value`], R, G, B, channels-last.
///
/// # Panics
///
/// When the crop or the buffer has the wrong size.
pub fn input_i8(crop: &RgbImage<'_>, input: &mut [i8]) {
    assert_eq!(
        (crop.width(), crop.height()),
        (CROP_SIZE, CROP_SIZE),
        "crop size"
    );
    let input = &mut input[..CROP_SIZE * CROP_SIZE * 3];
    for (value, &byte) in input.iter_mut().zip(crop.data()) {
        *value = input_value(byte);
    }
}

/// A layer's weights and plans.
#[derive(Clone, Copy, Debug)]
struct Layer<'m> {
    /// `[groups][taps][16]`.
    weights: &'m [i8],
    /// One per group.
    plans: &'m [Plan],
    /// Taps per output channel.
    taps: usize,
}

impl<'m> Layer<'m> {
    /// The layer as a 1x1 convolution over its taps.
    fn pointwise(&self) -> Pointwise<'m> {
        Pointwise {
            input: self.taps,
            weights: self.weights,
            plans: self.plans,
        }
    }

    /// The layer as a depthwise 3x3.
    fn depthwise(&self, stride: usize) -> Depthwise<'m> {
        Depthwise {
            weights: self.weights,
            plans: self.plans,
            stride,
        }
    }
}

/// A layer name built from a prefix and a suffix, without allocating.
struct Name {
    /// The bytes.
    bytes: [u8; 48],
    /// How many are used.
    len: usize,
}

impl Name {
    /// `prefix` then `suffix`.
    fn new(prefix: &str, suffix: &str) -> Self {
        let mut name = Self {
            bytes: [0; 48],
            len: 0,
        };
        for part in [prefix, suffix] {
            name.bytes[name.len..name.len + part.len()].copy_from_slice(part.as_bytes());
            name.len += part.len();
        }
        name
    }

    /// The name.
    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.len]).unwrap_or("")
    }
}

/// MFN_S8_V1 with every tensor found and every plan made: what
/// [`Model::forward`] runs.
#[derive(Clone, Copy, Debug)]
pub struct Model<'m> {
    /// The stem's 3x3 convolution, as a 1x1 over 32 taps.
    stem: Layer<'m>,
    /// The stem's depthwise layer.
    stem_depthwise: Layer<'m>,
    /// Each block's widening, depthwise and narrowing layer.
    blocks: [[Layer<'m>; 3]; 15],
    /// The head's widening 1x1.
    head_expand: Layer<'m>,
    /// The head's 7x7 depthwise layer.
    head_depthwise: Layer<'m>,
    /// The last 1x1, to the embedding.
    head_fc: Layer<'m>,
}

/// The working memory of one pass, lent by the caller. Every buffer
/// starts on a 16-byte boundary, or the kernels fall back to the scalar
/// model.
pub struct Scratch<'s> {
    /// The two tensors between blocks, [`TENSOR_LEN`] bytes each.
    pub tensors: [&'s mut [i8]; 2],
    /// The ring of wide rows: at least [`MIN_RING`] bytes, best
    /// [`FAST_RING`] or more, in internal RAM.
    pub ring: &'s mut [i8],
    /// A band of depthwise rows: at least [`MIN_FILTERED`] bytes, best
    /// [`FAST_FILTERED`], in internal RAM.
    pub filtered: &'s mut [i8],
    /// A band of output rows: at least [`MIN_STAGING`] bytes, best
    /// [`FAST_STAGING`], in internal RAM.
    pub staging: &'s mut [i8],
    /// The stem's columns, [`COLUMNS_LEN`] bytes.
    pub columns: &'s mut [i8],
}

impl Scratch<'_> {
    /// Check the lengths.
    ///
    /// # Panics
    ///
    /// When a buffer is too short.
    fn check(&self) {
        assert!(
            self.tensors.iter().all(|t| t.len() >= TENSOR_LEN),
            "two tensors of TENSOR_LEN bytes"
        );
        assert!(self.ring.len() >= MIN_RING, "a ring of MIN_RING bytes");
        assert!(
            self.filtered.len() >= MIN_FILTERED,
            "a band of MIN_FILTERED bytes"
        );
        assert!(
            self.staging.len() >= MIN_STAGING,
            "a band of MIN_STAGING bytes"
        );
        assert!(self.columns.len() >= COLUMNS_LEN, "COLUMNS_LEN columns");
    }
}

/// Read layer `name` from `blob` and make its plans in `plans`, which
/// shrinks by what the layer took.
///
/// # Panics
///
/// When a tensor is missing or does not match the others.
fn compile_layer<'m>(blob: &Blob<'m>, name: &str, plans: &mut &'m mut [Plan]) -> Layer<'m> {
    let tensor = |suffix: &str| {
        let full = Name::new(name, suffix);
        blob.get(full.as_str())
            .unwrap_or_else(|| panic!("the MFN weights have no tensor {}", full.as_str()))
    };
    let weight = tensor(".weight");
    let &[groups, taps, lanes] = weight.shape() else {
        panic!("{name}: weights of rank {}", weight.shape().len())
    };
    assert_eq!(lanes, LANES, "{name}: sixteen channels per group");
    let weights = weight.i8_slice();
    let bias = tensor(".bias");
    let shift = tensor(".shift");
    assert_eq!(bias.element_count(), groups * LANES, "{name}: biases");
    assert_eq!(shift.element_count(), groups, "{name}: shifts");
    let prelu_tensors = {
        let alpha = Name::new(name, ".alpha");
        blob.get(alpha.as_str())
            .map(|alpha| (alpha.i8_slice(), tensor(".prelu")))
    };
    let (mine, rest) = core::mem::take(plans).split_at_mut(groups);
    *plans = rest;
    let mut biases = bias.i32s();
    let mut shifts = shift.i32s();
    let mut prelu_shifts = prelu_tensors.as_ref().map(|(_, p)| p.i32s());
    for (g, plan) in mine.iter_mut().enumerate() {
        let group_bias: [i32; LANES] =
            core::array::from_fn(|_| biases.next().expect("the biases are counted"));
        let shift = shifts.next().expect("the shifts are counted") as u32;
        let prelu = match (&prelu_tensors, prelu_shifts.as_mut()) {
            (Some((alpha, _)), Some(pairs)) => {
                let mut slopes = [0i8; LANES];
                slopes.copy_from_slice(&alpha[g * LANES..(g + 1) * LANES]);
                let positive = pairs.next().expect("two PReLU shifts per group") as u32;
                let negative = pairs.next().expect("two PReLU shifts per group") as u32;
                Some(GroupPrelu {
                    alpha: slopes,
                    positive,
                    shift: negative,
                })
            }
            _ => None,
        };
        *plan = Plan::new(&group_bias, shift, prelu.as_ref());
    }
    Layer {
        weights,
        plans: mine,
        taps,
    }
}

impl<'m> Model<'m> {
    /// Find every tensor of `blob` (from `facekit import-espdl`) and make
    /// every plan into `plans` ([`MODEL_PLANS`] of them). The weights stay
    /// where `blob` has them and are read from there; they should start
    /// on a 16-byte boundary (`include_fkb!`, or a copy so aligned).
    ///
    /// # Panics
    ///
    /// When a tensor is missing or does not fit, the input's exponent is
    /// not [`INPUT_EXPONENT`], or `plans` is too short.
    pub fn compile(blob: &Blob<'m>, plans: &'m mut [Plan]) -> Self {
        assert!(plans.len() >= MODEL_PLANS, "MODEL_PLANS plans");
        let exponent = blob
            .get("input.exponent")
            .expect("the MFN weights have no input.exponent")
            .i32s()
            .next();
        assert_eq!(exponent, Some(INPUT_EXPONENT), "the input's exponent");
        let mut plans = plans;
        let stem = compile_layer(blob, "conv_1", &mut plans);
        assert_eq!(stem.taps, STEM_TAPS, "the stem's taps");
        let stem_depthwise = compile_layer(blob, "conv_2_dw", &mut plans);
        let blocks = BLOCKS.map(|(prefix, _, _)| {
            ["_conv_sep", "_conv_dw", "_conv_proj"]
                .map(|suffix| compile_layer(blob, Name::new(prefix, suffix).as_str(), &mut plans))
        });
        let head_expand = compile_layer(blob, "conv_6sep", &mut plans);
        let head_depthwise = compile_layer(blob, "conv_6dw7_7", &mut plans);
        assert_eq!(head_depthwise.taps, HEAD_SIDE * HEAD_SIDE, "the 7x7 taps");
        let head_fc = compile_layer(blob, "fc1", &mut plans);
        assert_eq!(head_fc.pointwise().output(), EMBEDDING_LEN, "the embedding");
        Self {
            stem,
            stem_depthwise,
            blocks,
            head_expand,
            head_depthwise,
            head_fc,
        }
    }

    /// Block `index` as [`block`] runs it.
    fn block(&self, index: usize) -> Block<'m> {
        let (_, side, residual) = BLOCKS[index];
        let [expand, depthwise, project] = self.blocks[index];
        Block {
            height: side,
            width: side,
            expand: expand.pointwise(),
            depthwise: depthwise.depthwise(if residual { 1 } else { 2 }),
            project: project.pointwise(),
            residual,
        }
    }

    /// The products of one pass.
    pub fn products(&self) -> usize {
        let stem = STEM_SIDE * STEM_SIDE * STEM_CHANNELS * (27 + 9);
        let blocks: usize = (0..BLOCKS.len()).map(|b| self.block(b).products()).sum();
        let head = HEAD_SIDE * HEAD_SIDE * 512 * (128 + 1) + 512 * EMBEDDING_LEN;
        stem + blocks + head
    }

    /// The embedding of `input` (the quantized crop, see [`input_i8`]) in
    /// `embedding`: [`EMBEDDING_LEN`] values, not normalized.
    ///
    /// # Panics
    ///
    /// When a buffer has the wrong size; see [`Scratch`].
    pub fn forward(&self, input: &[i8], scratch: Scratch<'_>, embedding: &mut [i8]) {
        self.forward_traced(input, scratch, embedding, |_| {});
    }

    /// [`Model::forward`], calling `trace` with the name of each part
    /// when it is done: `stem`, each block's prefix, `head`.
    ///
    /// # Panics
    ///
    /// As [`Model::forward`].
    pub fn forward_traced(
        &self,
        input: &[i8],
        scratch: Scratch<'_>,
        embedding: &mut [i8],
        mut trace: impl FnMut(&str),
    ) {
        scratch.check();
        assert_eq!(input.len(), CROP_SIZE * CROP_SIZE * 3, "the input crop");
        assert_eq!(embedding.len(), EMBEDDING_LEN, "the embedding");
        let Scratch {
            tensors: [first, second],
            ring,
            filtered,
            staging,
            columns,
        } = scratch;
        let (mut current, mut next) = (first, second);

        self.stem(input, ring, columns, current);
        trace("stem");

        for (index, (prefix, ..)) in BLOCKS.iter().enumerate() {
            let block = self.block(index);
            let band = block.band_for(ring.len(), filtered.len(), staging.len());
            assert!(band > 0, "the internal buffers hold one band of {prefix}");
            let input_len = block.height * block.width * block.channels();
            block::run_banded(
                &block,
                band,
                &current[..input_len],
                ring,
                filtered,
                staging,
                &mut next[..block.output_len()],
            );
            core::mem::swap(&mut current, &mut next);
            trace(prefix);
        }

        self.head(current, next, columns, staging, embedding);
        trace("head");
    }

    /// The stem: the 3x3 convolution with stride 2 one output row at a
    /// time into `ring`, as a 1x1 over the row's columns, and the
    /// depthwise layer over the ring into `output`, in bands as tall as
    /// the ring allows.
    fn stem(&self, input: &[i8], ring: &mut [i8], columns: &mut [i8], output: &mut [i8]) {
        let convolution = self.stem.pointwise();
        let depthwise = self.stem_depthwise.depthwise(1);
        let row = STEM_SIDE * STEM_CHANNELS;
        let band = (ring.len() / row - 2).min(STEM_SIDE);
        let slots = band + 2;
        let ring = &mut ring[..slots * row];
        let columns = &mut columns[..COLUMNS_LEN];
        let mut ready = 0;
        for first in (0..STEM_SIDE).step_by(band) {
            let rows = band.min(STEM_SIDE - first);
            let last = (first + rows).min(STEM_SIDE - 1);
            while ready <= last {
                stem_columns(input, ready, columns);
                let slot = ready % slots;
                s8::pointwise(
                    &convolution,
                    Store::Write,
                    columns,
                    &mut ring[slot * row..(slot + 1) * row],
                );
                ready += 1;
            }
            let ring = &*ring;
            let slot = |k: usize| &ring[k % slots * row..(k % slots + 1) * row];
            for r in first..first + rows {
                let rows = [
                    r.checked_sub(1).map(slot),
                    Some(slot(r)),
                    (r + 1 < STEM_SIDE).then(|| slot(r + 1)),
                ];
                s8::depthwise_row(
                    &depthwise,
                    rows,
                    STEM_SIDE,
                    &mut output[r * row..(r + 1) * row],
                );
            }
        }
    }

    /// The head on the 7x7x128 `input`: the widening 1x1 into `wide`, the
    /// 7x7 depthwise layer into `columns`, the last 1x1 into `staging`,
    /// copied to `embedding`.
    fn head(
        &self,
        input: &[i8],
        wide: &mut [i8],
        columns: &mut [i8],
        staging: &mut [i8],
        embedding: &mut [i8],
    ) {
        let expand = self.head_expand.pointwise();
        let pixels = HEAD_SIDE * HEAD_SIDE;
        let channels = expand.output();
        let wide = &mut wide[..pixels * channels];
        s8::pointwise(&expand, Store::Write, &input[..pixels * expand.input], wide);
        let pooled = &mut columns[..channels];
        let taps = self.head_depthwise.taps;
        for (c, target) in pooled.iter_mut().enumerate() {
            let filter = &self.head_depthwise.weights[c / LANES * taps * LANES..];
            let lane = c % LANES;
            let mut sum = 0i32;
            for t in 0..taps {
                sum += i32::from(wide[t * channels + c]) * i32::from(filter[t * LANES + lane]);
            }
            s8::model::finish(
                &self.head_depthwise.plans[c / LANES],
                lane,
                sum,
                Store::Write,
                target,
            );
        }
        let fc = self.head_fc.pointwise();
        let out = &mut staging[..EMBEDDING_LEN];
        s8::pointwise(&fc, Store::Write, pooled, out);
        embedding.copy_from_slice(out);
    }
}

/// The columns of stem output row `oy`: for each of its 56 pixels, the
/// 3x3x3 input values around `(2 ox, 2 oy)` in the order of the weights
/// (row, column, channel), zero outside the image, then five zeros.
fn stem_columns(input: &[i8], oy: usize, columns: &mut [i8]) {
    let side = CROP_SIZE;
    for (ox, pixel) in columns.chunks_exact_mut(STEM_TAPS).enumerate() {
        pixel.fill(0);
        for ky in 0..3 {
            let Some(y) = (2 * oy + ky).checked_sub(1).filter(|&y| y < side) else {
                continue;
            };
            for kx in 0..3 {
                let Some(x) = (2 * ox + kx).checked_sub(1).filter(|&x| x < side) else {
                    continue;
                };
                let at = (ky * 3 + kx) * 3;
                pixel[at..at + 3].copy_from_slice(&input[(y * side + x) * 3..][..3]);
            }
        }
    }
}
