//! The 8-bit lane pipeline: sixteen `i8 x i8` products per instruction,
//! for networks quantized the way Espressif's ESP-DL quantizes them.
//!
//! [`lanes`](super::lanes) keeps every tensor `i16` and multiplies eight
//! 16-bit lanes per instruction. ESP-DL quantizes every tensor to `i8`
//! with one power-of-two scale per tensor, weights included; Espressif's
//! MFN_S8_V1 face recognizer ([`mfn`](super::mfn)) is such a network. Its
//! values fit the vector unit's 8-bit mode: `ee.vmulas.s8.qacc`
//! multiplies sixteen pairs at once into sixteen 20-bit accumulator
//! lanes, twice the products of the 16-bit mode, from half the bytes.
//!
//! The kernels: the 1x1 convolution ([`pointwise`], 93 percent of
//! MFN_S8_V1's products) and the depthwise 3x3 ([`depthwise_row`]), both
//! with the PReLU that follows them, and the network's building block
//! ([`block`]): the widening 1x1, the depthwise 3x3 and the narrowing
//! 1x1, with or without the residual add. A benchmark binary measured
//! them on the board (builds `mfnbench-1` to `-4`, see
//! `docs/face_id/performance.md`); it is no longer in the repository.
//!
//! # Arithmetic
//!
//! Output channel `o` of a pixel is
//!
//! ```text
//! out[o] = clamp((bias[o] + 2^(shift - 1) + sum x * w) >> shift, -128, 127)
//! ```
//!
//! a division by `2^shift`, rounded half up, then saturated. The
//! accumulator starts from the first two terms (the group's [`Plan`]),
//! the products accumulate, and `ee.srcmb.s8.qacc` shifts and saturates.
//! The lanes are 20 bits wide and saturate (measured on the board); the
//! sums of MFN_S8_V1 stay below 87,000 on every LFW photo, far inside
//! ±524,287. [`model`] computes them exactly.
//!
//! A PReLU after the convolution works on that `i8` value `v`, as
//! ESP-DL's separate PReLU layer does: `v * 2^positive` (saturated) when
//! `v >= 0`, else `v * alpha[o]` shifted right by its own shift, rounded
//! half up, saturated. Its `positive` shift is 0 or 1 in MFN_S8_V1, its
//! negative one 6 to 8.
//!
//! The shift and the PReLU belong to a group of sixteen channels, not to
//! the layer: ESP-DL splits a few of MFN_S8_V1's layers into two halves
//! with scales of their own and concatenates them, and here such a layer
//! is one layer whose groups differ.
//!
//! A residual add ([`Store::Add`]) adds the result to what the output
//! already holds, saturating: MFN_S8_V1's adds have the same scale on
//! both inputs and the output.
//!
//! # Layout
//!
//! Activations are `i8`, channels-last, as everywhere in this crate. The
//! weights of a 1x1 layer are `[output / 16][input][16]`: for each input
//! channel, the sixteen output channels of a group side by side; a
//! depthwise 3x3 layer's are `[channels / 16][3][3][16]`. Both are
//! ESP-DL's own `N16HWC16` layout. Channel counts are multiples of 16;
//! the weights, the activations and the output start on a 16-byte
//! boundary.

pub mod block;
pub mod model;

/// Channels in a group: the lanes of the accumulator in 8-bit mode.
pub const LANES: usize = 16;

/// The bits of one accumulator lane in 8-bit mode.
pub const LANE_BITS: u32 = 20;

/// Bytes of input pixels that a batch keeps in the data cache while
/// every group of output channels passes over them.
pub const BATCH_BYTES: usize = 16 * 1024;

/// The bytes of a [`Plan`], as the assembly steps from one to the next.
pub const PLAN_BYTES: usize = 224;

/// What one group of sixteen output channels needs, laid out as the
/// assembly reads it (224 bytes, 16-byte aligned).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C, align(16))]
pub struct Plan {
    /// The accumulator image the sums start from: each lane's bias plus
    /// half of `2^shift`; see [`encode_lanes`]. Offset 0.
    pub image: [u32; 16],
    /// The accumulator image of the PReLU's negative branch: half of
    /// `2^prelu_shift` in every lane. Offset 64.
    pub prelu_image: [u32; 16],
    /// The PReLU's slope of each channel. Offset 128.
    pub alpha: [i8; LANES],
    /// The PReLU's left shift of non-negative values: 0 or 1. Offset 144.
    pub positive: u32,
    /// The PReLU's right shift of negative values times `alpha`. Offset
    /// 148.
    pub prelu_shift: u32,
    /// The right shift from product units to output units. Offset 152.
    pub shift: u32,
    /// 1 when the group has a PReLU, else 0. Offset 156.
    pub prelu: u32,
    /// The bias of each channel, in product units, for the scalar model.
    /// Offset 160.
    pub bias: [i32; LANES],
}

/// One group's PReLU: see the module's arithmetic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GroupPrelu {
    /// The slope of each channel, applied to negative values.
    pub alpha: [i8; LANES],
    /// The left shift of non-negative values: 0 or 1.
    pub positive: u32,
    /// The right shift of a negative value times its slope.
    pub shift: u32,
}

impl Plan {
    /// A plan that starts every lane from zero, without a PReLU.
    pub const ZERO: Self = Self {
        image: [0; 16],
        prelu_image: [0; 16],
        alpha: [0; LANES],
        positive: 0,
        prelu_shift: 0,
        shift: 0,
        prelu: 0,
        bias: [0; LANES],
    };

    /// The plan for sixteen channels with `bias` (in product units) whose
    /// sums are shifted right by `shift`, rounding half up, then go
    /// through `prelu`.
    ///
    /// # Panics
    ///
    /// When the PReLU's positive shift is above 1, or a bias does not fit
    /// a 20-bit lane.
    pub fn new(bias: &[i32; LANES], shift: u32, prelu: Option<&GroupPrelu>) -> Self {
        let half = model::half(shift);
        let limit = 1 << (LANE_BITS - 1);
        assert!(
            bias.iter().all(|&b| (-limit..limit - half).contains(&b)),
            "biases fit a 20-bit lane"
        );
        let mut plan = Self {
            image: encode_lanes(&bias.map(|b| b + half)),
            shift,
            bias: *bias,
            ..Self::ZERO
        };
        if let Some(prelu) = prelu {
            assert!(prelu.positive <= 1, "a positive shift of 0 or 1");
            plan.prelu_image = encode_lanes(&[model::half(prelu.shift); LANES]);
            plan.alpha = prelu.alpha;
            plan.positive = prelu.positive;
            plan.prelu_shift = prelu.shift;
            plan.prelu = 1;
        }
        plan
    }

    /// The group's PReLU, if it has one.
    pub fn group_prelu(&self) -> Option<GroupPrelu> {
        (self.prelu != 0).then_some(GroupPrelu {
            alpha: self.alpha,
            positive: self.positive,
            shift: self.prelu_shift,
        })
    }
}

/// Sixteen 20-bit lane values as the accumulator stores and loads them in
/// 8-bit mode: lanes 0 to 7 in the 160 bits of words 0..5, lane `i` at
/// bit `20 i`, two's complement; lanes 8 to 15 likewise in words 8..13.
/// Bits above 20 are dropped. (Measured on the board.)
pub fn encode_lanes(values: &[i32; LANES]) -> [u32; 16] {
    let mut words = [0u32; 16];
    for (half, base) in [0usize, 8].into_iter().enumerate() {
        let mut bytes = [0u8; 20];
        for i in 0..8 {
            let value = (values[half * 8 + i] as u32) & ((1 << LANE_BITS) - 1);
            let at = LANE_BITS as usize * i;
            let shifted = value << (at % 8);
            for (k, byte) in shifted.to_le_bytes().iter().take(3).enumerate() {
                bytes[at / 8 + k] |= byte;
            }
        }
        for (w, word) in bytes.chunks_exact(4).enumerate() {
            words[base + w] = u32::from_le_bytes([word[0], word[1], word[2], word[3]]);
        }
    }
    words
}

/// The sixteen lane values of a stored accumulator, sign-extended from
/// 20 bits: the inverse of [`encode_lanes`].
pub fn decode_lanes(words: &[u32; 16]) -> [i32; LANES] {
    let mut values = [0i32; LANES];
    for (half, base) in [0usize, 8].into_iter().enumerate() {
        let mut bytes = [0u8; 20];
        for (w, chunk) in bytes.chunks_exact_mut(4).enumerate() {
            chunk.copy_from_slice(&words[base + w].to_le_bytes());
        }
        for i in 0..8 {
            let at = LANE_BITS as usize * i;
            let low = u32::from_le_bytes([bytes[at / 8], bytes[at / 8 + 1], bytes[at / 8 + 2], 0]);
            let raw = (low >> (at % 8)) & ((1 << LANE_BITS) - 1);
            // Sign-extend from bit 19.
            values[half * 8 + i] = ((raw << (32 - LANE_BITS)) as i32) >> (32 - LANE_BITS);
        }
    }
    values
}

/// A PReLU with one shift for every channel of a layer: what [`plans`]
/// splits into groups.
#[derive(Clone, Copy, Debug)]
pub struct Prelu<'a> {
    /// The slope of each channel, applied to negative values.
    pub alpha: &'a [i8],
    /// The left shift of non-negative values: 0 or 1.
    pub positive: u32,
    /// The right shift of a negative value times its slope.
    pub shift: u32,
}

/// Fill `plans` (one per sixteen channels) for `bias`, one `shift` for
/// every group, and `prelu`.
///
/// # Panics
///
/// When `bias` (or the PReLU's slopes) are not sixteen values per plan.
pub fn plans(bias: &[i32], shift: u32, prelu: Option<&Prelu<'_>>, plans: &mut [Plan]) {
    assert_eq!(bias.len(), plans.len() * LANES, "sixteen biases per plan");
    if let Some(prelu) = prelu {
        assert_eq!(prelu.alpha.len(), bias.len(), "a slope per channel");
    }
    for (g, (plan, bias)) in plans.iter_mut().zip(bias.chunks_exact(LANES)).enumerate() {
        let mut group = [0i32; LANES];
        group.copy_from_slice(bias);
        let group_prelu = prelu.map(|p| {
            let mut alpha = [0i8; LANES];
            alpha.copy_from_slice(&p.alpha[g * LANES..(g + 1) * LANES]);
            GroupPrelu {
                alpha,
                positive: p.positive,
                shift: p.shift,
            }
        });
        *plan = Plan::new(&group, shift, group_prelu.as_ref());
    }
}

/// Whether every plan has a PReLU (`Some(true)`), none has
/// (`Some(false)`), or they differ (`None`).
fn uniform_prelu(plans: &[Plan]) -> Option<bool> {
    let first = plans.first().is_some_and(|plan| plan.prelu != 0);
    plans
        .iter()
        .all(|plan| (plan.prelu != 0) == first)
        .then_some(first)
}

/// A 1x1 convolution as this module runs it.
#[derive(Clone, Copy, Debug)]
pub struct Pointwise<'a> {
    /// Input channels: a multiple of 16.
    pub input: usize,
    /// The weights, `[output / 16][input][16]`.
    pub weights: &'a [i8],
    /// One plan per group of sixteen output channels: biases, shifts,
    /// PReLU.
    pub plans: &'a [Plan],
}

impl Pointwise<'_> {
    /// Output channels.
    pub fn output(&self) -> usize {
        self.plans.len() * LANES
    }
}

/// A depthwise 3x3 convolution with padding 1, as this module runs it.
#[derive(Clone, Copy, Debug)]
pub struct Depthwise<'a> {
    /// The weights, `[channels / 16][3][3][16]`.
    pub weights: &'a [i8],
    /// One plan per group of sixteen channels.
    pub plans: &'a [Plan],
    /// The step between output pixels in the input: 1 or 2.
    pub stride: usize,
}

impl Depthwise<'_> {
    /// Channels.
    pub fn channels(&self) -> usize {
        self.plans.len() * LANES
    }

    /// The output width (or height) for an input `size` wide (or high).
    pub fn output_size(&self, size: usize) -> usize {
        (size - 1) / self.stride + 1
    }
}

/// How a kernel's results reach the output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Store {
    /// Write them.
    Write,
    /// Add them to what is there, saturating: the residual connection.
    Add,
}

/// Kernel calls that ran the scalar model on a board that has the vector
/// unit, because an operand did not fit the instructions' rules.
static FALLBACKS: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

/// How many kernel calls fell back to the scalar model on the board.
pub fn fallbacks() -> usize {
    FALLBACKS.load(core::sync::atomic::Ordering::Relaxed)
}

/// Count one fallback; called only where the vector unit exists.
#[cfg(target_arch = "xtensa")]
fn note_fallback() {
    FALLBACKS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
}

/// The 1x1 convolution `layer` of every pixel of `input` (`layer.input`
/// values each) into `output` (`layer.output()` values per pixel), as
/// `store` says. On the board it runs on the vector unit; elsewhere, or
/// when a buffer is off a 16-byte boundary, it runs
/// [`model::pointwise`], with the same result.
///
/// # Panics
///
/// When the shapes do not match, a channel count is not a multiple of
/// 16, or some groups have a PReLU and others not.
pub fn pointwise(layer: &Pointwise<'_>, store: Store, input: &[i8], output: &mut [i8]) {
    let (channels, outputs) = (layer.input, layer.output());
    assert!(
        channels >= LANES && channels.is_multiple_of(LANES) && outputs > 0,
        "channel counts must be multiples of 16"
    );
    assert_eq!(layer.weights.len(), channels * outputs, "weights");
    assert!(input.len().is_multiple_of(channels), "whole input pixels");
    assert_eq!(
        output.len(),
        input.len() / channels * outputs,
        "output size"
    );
    let prelu = uniform_prelu(layer.plans).expect("every group with a PReLU or none");
    #[cfg(target_arch = "xtensa")]
    {
        if arch::pointwise(layer, prelu, store, input, output) {
            return;
        }
        note_fallback();
    }
    let _ = prelu;
    model::pointwise(layer, store, input, output);
}

/// One output row of the depthwise convolution `layer`: `rows` are the
/// input rows above, at and below the centre row (`width` pixels each;
/// `None` outside the image, which counts as zeros), `output` the row's
/// [`Depthwise::output_size`] pixels. On the board it runs on the
/// vector unit; elsewhere, or when a buffer is off a 16-byte boundary,
/// it runs [`model::depthwise_row`], with the same result.
///
/// # Panics
///
/// When the shapes do not match, or some groups have a PReLU and others
/// not.
pub fn depthwise_row(
    layer: &Depthwise<'_>,
    rows: [Option<&[i8]>; 3],
    width: usize,
    output: &mut [i8],
) {
    let channels = layer.channels();
    assert!(channels > 0, "at least one group");
    assert_eq!(layer.weights.len(), channels * 9, "weights");
    assert!(
        rows.iter()
            .flatten()
            .all(|row| row.len() == width * channels),
        "input rows"
    );
    assert_eq!(
        output.len(),
        layer.output_size(width) * channels,
        "output row"
    );
    let prelu = uniform_prelu(layer.plans).expect("every group with a PReLU or none");
    #[cfg(target_arch = "xtensa")]
    {
        if arch::depthwise_row(layer, prelu, rows, width, output) {
            return;
        }
        note_fallback();
    }
    let _ = prelu;
    model::depthwise_row(layer, rows, width, output);
}

#[cfg(target_arch = "xtensa")]
mod arch;

#[cfg(target_arch = "xtensa")]
pub mod probe;
