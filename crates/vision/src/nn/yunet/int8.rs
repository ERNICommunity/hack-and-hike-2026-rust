//! YuNet in integers: the forward pass the firmware runs.
//!
//! The same graph as the `f32` pass in [`super`], unit for unit and buffer
//! for buffer, but every convolution runs on integer input with `i8`
//! weights (see [`quant`](super::super::quant)) and writes its result as
//! integers again, quantized with the mapping of the tensor it produces.
//! Only the outputs of the heads are `f32`. Each stage of this pass
//! compares with the same stage of the `f32` pass to within the
//! quantization noise; the tests measure that as a signal-to-noise ratio.
//!
//! Every activation is **16-bit** with one symmetric mapping per tensor
//! ([`Granularity::Wide`](super::super::quant::Granularity::Wide)): the
//! Relu, MaxPool and Add outputs, the 1x1 output inside each unit and the
//! 1x1 output inside each head. The detector is small (about four
//! million multiply-accumulates per frame), so the wider activations cost
//! little time, and they keep every stage within a fraction of a percent
//! of the `f32` pass. Only the input image is 8-bit, with the fixed
//! mapping [`INPUT_QUANT`]; the weights are 8-bit with one scale per
//! output channel.
//!
//! # The weights file
//!
//! `facekit quantize` turns the `f32` weights file into an integer one
//! that this pass reads through [`Weights`]:
//!
//! - Every convolution's `<name>.weight` becomes an `i8` tensor of the same
//!   layout (read with [`Weights::get_i8`]) plus an `f32` tensor
//!   `<name>.scales` with one scale per output channel. `<name>.bias`
//!   stays `f32`.
//! - Every activation tensor that stays quantized has an `f32` tensor
//!   `q.<node>` = `[scale, zero point]` (layout `Q16`, zero point 0),
//!   where `node` is the ONNX node that produces the tensor: the Relu and
//!   Add outputs (`Relu_1`, `Add_44`, ...), the 1x1 output inside each
//!   unit (`Conv_2`, `Conv_6`, ...) and the 1x1 output inside each head
//!   (`Conv_54`, `Conv_60`, ...). These are the tensors `forward_probed`
//!   of the `f32` pass probes. A MaxPool output keeps its input's mapping
//!   and needs no entry.
//!
//! # Memory
//!
//! The caller lends two slices, split by [`Scratch::new`]:
//!
//! - `i16`, [`SCRATCH_I16_LEN`] values (160 KiB): the two work buffers
//!   and the three feature maps of the `f32` pass's plan, plus one work
//!   buffer for the 1x1 output inside a unit or a head.
//! - `f32`, [`F32_SCRATCH_LEN`] values (104 KiB): the head outputs
//!   (8 KiB) and a 96 KiB work buffer that only tracing uses, for the
//!   dequantized copy of each traced tensor.
//!
//! That is 264 KiB in all, or 168 KiB without the trace buffer: roughly
//! double what an all-8-bit version would need (its maps and work buffers
//! would take 80 KiB), because the activations are twice as wide, and
//! about the same as the `f32` pass's 235 KiB.

use super::{
    super::{
        Activation, Shape, Weights,
        quant::{self, Output, QWeight, Quant, simd},
        sigmoid,
    },
    ANCHORS, HEAD_VALUES, HeadBuffers, Heads, INPUT_SHAPE, LEVELS, Level, MAP_8, MAP_16, MAP_32,
    Name, UNITS, Unit, WORK_LEN, take,
};

/// The mapping of the input image: a byte 0..255 becomes `byte - 128`.
/// `detect::detector_input_i8` writes it directly.
pub const INPUT_QUANT: Quant = Quant {
    scale: 1.0,
    zero_point: -128,
};
/// The first convolution's output channels.
const STEM_FILTERS: usize = 16;
/// The input's channels once widened for the vector unit: eight 16-bit
/// values are one 128-bit register. The three colour channels come first,
/// the other five are zero.
const INPUT_PADDED: usize = 8;
/// The widened input, in values.
const INPUT_WIDE_LEN: usize = INPUT_SHAPE.pixels() * INPUT_PADDED;
/// The length of the `i16` scratch slice: the two work buffers, the three
/// feature maps, one work buffer for the largest 1x1 output inside a
/// unit (the stem unit's, 32x48x16, which is also bigger than any head's),
/// and the widened input.
pub const SCRATCH_I16_LEN: usize =
    3 * WORK_LEN + MAP_8.len() + MAP_16.len() + MAP_32.len() + INPUT_WIDE_LEN;
/// The `f32` work buffer: the largest traced tensor.
const WORK_F32_LEN: usize = WORK_LEN;
/// The length of the `f32` scratch slice: the head outputs and the `f32`
/// work buffer.
pub const F32_SCRATCH_LEN: usize = ANCHORS * HEAD_VALUES + WORK_F32_LEN;

/// The scratch memory of one integer forward pass, split into its buffers.
///
/// Make one with [`Scratch::new`]. The forward pass takes it and the
/// returned [`Heads`] point into its `f32` part.
pub struct Scratch<'a> {
    /// A work buffer for the stem and the first stage.
    work_a: &'a mut [i16],
    /// The other work buffer: units alternate between the two.
    work_b: &'a mut [i16],
    /// The 8x map: the backbone's, then the neck's sum, then the feature map.
    map_8: &'a mut [i16],
    /// The 16x map, likewise.
    map_16: &'a mut [i16],
    /// The 32x map: the backbone's, then the neck's feature map.
    map_32: &'a mut [i16],
    /// The 1x1 result inside a unit or a head.
    mid: &'a mut [i16],
    /// The input widened to eight channels, for the first convolution.
    input_wide: &'a mut [i16],
    /// The `f32` work buffer, for tracing.
    work_f32: &'a mut [f32],
    /// The outputs of the heads, per stride.
    heads: [HeadBuffers<'a>; 3],
}

impl<'a> Scratch<'a> {
    /// Split the caller's memory: `i16s` has at least [`SCRATCH_I16_LEN`]
    /// values and `f32s` at least [`F32_SCRATCH_LEN`].
    ///
    /// # Panics
    ///
    /// When a slice is too short.
    pub fn new(i16s: &'a mut [i16], f32s: &'a mut [f32]) -> Self {
        assert!(
            i16s.len() >= SCRATCH_I16_LEN,
            "yunet int8 i16 scratch: {} values, need {SCRATCH_I16_LEN}",
            i16s.len()
        );
        assert!(
            f32s.len() >= F32_SCRATCH_LEN,
            "yunet int8 f32 scratch: {} values, need {F32_SCRATCH_LEN}",
            f32s.len()
        );
        let mut rest = i16s;
        let work_a = take(&mut rest, WORK_LEN);
        let work_b = take(&mut rest, WORK_LEN);
        let map_8 = take(&mut rest, MAP_8.len());
        let map_16 = take(&mut rest, MAP_16.len());
        let map_32 = take(&mut rest, MAP_32.len());
        let mid = take(&mut rest, WORK_LEN);
        let input_wide = take(&mut rest, INPUT_WIDE_LEN);
        let mut rest = f32s;
        let heads = HeadBuffers::take_all(&mut rest);
        let work_f32 = take(&mut rest, WORK_F32_LEN);
        Self {
            work_a,
            work_b,
            map_8,
            map_16,
            map_32,
            mid,
            input_wide,
            work_f32,
            heads,
        }
    }
}

/// Where traced tensors go: dequantized into the `f32` work buffer, then
/// to the callback. `None` means nothing is traced.
struct Tracer<F> {
    /// The caller's callback, if any.
    trace: Option<F>,
}

impl<F: FnMut(&str, Shape, &[f32])> Tracer<F> {
    /// Trace `data`, an `i16` tensor of `shape` mapped by `quant`, under
    /// the name `node`, using `work` for the dequantized copy.
    fn emit(&mut self, work: &mut [f32], node: &str, shape: Shape, data: &[i16], quant: Quant) {
        let Some(trace) = &mut self.trace else {
            return;
        };
        let values = &mut work[..shape.len()];
        for (real, &q) in values.iter_mut().zip(view(data, shape)) {
            *real = quant.dequantize16(q);
        }
        trace(node, shape, values);
    }
}

/// The 16-bit mapping of the activation that `node` produces.
fn quant_of(weights: &impl Weights, node: &str) -> Quant {
    Quant::from_pair(weights.get(Name::join(&["q.", node]).as_str()))
}

/// The integer weight, its scales and its bias of the convolution called
/// `prefix`.
fn tensors<'w>(weights: &'w impl Weights, prefix: &str) -> QWeight<'w> {
    let weight_name = Name::join(&[prefix, ".weight"]);
    QWeight {
        data: weights.get_i8(weight_name.as_str()),
        scales: weights.get(Name::join(&[prefix, ".scales"]).as_str()),
        bias: weights.get(Name::join(&[prefix, ".bias"]).as_str()),
        packed: weights.packed(weight_name.as_str()),
    }
}

/// The first `shape.len()` values of `buffer`: the activation it holds.
fn view<T>(buffer: &[T], shape: Shape) -> &[T] {
    &buffer[..shape.len()]
}

/// The name of a head convolution: `bbox_head.multi_level_<kind>.<level>.<conv>`.
fn head_name(kind: &str, level: &Level, conv: &str) -> Name {
    Name::join(&["bbox_head.multi_level_", kind, ".", level.name, ".", conv])
}

/// One unit of the backbone or the neck, as in the `f32` pass: the 1x1
/// convolution, quantized with its node's mapping into `work`, then the
/// depthwise 3x3 convolution with Relu, quantized with the Relu node's
/// mapping back into `data`.
///
/// `data` holds the input of `shape`, mapped by `quant`. Returns the
/// output's shape and mapping.
fn unit(
    weights: &impl Weights,
    unit: &Unit,
    data: &mut [i16],
    quant: Quant,
    shape: Shape,
    work: &mut [i16],
) -> (Shape, Quant) {
    let mid_quant = quant_of(weights, unit.pointwise_node);
    let mid = quant::conv2d_16(
        view(data, shape),
        quant,
        shape,
        &tensors(weights, unit.pointwise),
        1,
        1,
        0,
        Activation::None,
        Output::I16(work, mid_quant),
    );
    let out_quant = quant_of(weights, unit.relu_node);
    let out = quant::depthwise_16(
        view(work, mid),
        mid_quant,
        mid,
        &tensors(weights, unit.depthwise_node),
        3,
        1,
        1,
        Activation::Relu,
        Output::I16(data, out_quant),
    );
    (out, out_quant)
}

/// 2x2 max pooling on `i16`, stride 2, like `nn::max_pool_2x2`. The
/// mapping does not change: the maximum of quantized values is the
/// quantized maximum.
///
/// # Panics
///
/// When the slices do not match the shapes.
pub fn max_pool_2x2_i16(input: &[i16], shape: Shape, output: &mut [i16]) -> Shape {
    let out = Shape::new(shape.height / 2, shape.width / 2, shape.channels);
    assert_eq!(input.len(), shape.len(), "max_pool_2x2_i16 input");
    assert!(output.len() >= out.len(), "max_pool_2x2_i16 output buffer");
    for oy in 0..out.height {
        for ox in 0..out.width {
            for c in 0..shape.channels {
                let mut max = i16::MIN;
                for (dy, dx) in [(0, 0), (0, 1), (1, 0), (1, 1)] {
                    max = max.max(input[shape.offset(ox * 2 + dx, oy * 2 + dy) + c]);
                }
                output[out.offset(ox, oy) + c] = max;
            }
        }
    }
    out
}

/// Add `small`, enlarged 2x by repeating each pixel, to `target`, in place,
/// like `nn::upsample_2x_add` but on `i16` tensors: each pair of values is
/// dequantized with its own mapping, added, and the sum quantized with
/// `out_quant`, which is `target`'s mapping from then on. `target` has
/// twice the rows and columns of `small`.
///
/// # Panics
///
/// When the shapes do not match.
pub fn upsample_2x_add_i16(
    small: &[i16],
    small_quant: Quant,
    small_shape: Shape,
    target: &mut [i16],
    target_quant: Quant,
    target_shape: Shape,
    out_quant: Quant,
) {
    assert_eq!(small.len(), small_shape.len(), "upsample_2x_add_i16 small");
    assert_eq!(
        target.len(),
        target_shape.len(),
        "upsample_2x_add_i16 target"
    );
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
        "upsample_2x_add_i16 shapes"
    );
    let channels = small_shape.channels;
    for y in 0..target_shape.height {
        for x in 0..target_shape.width {
            let source = &small
                [small_shape.offset(x / 2, y / 2)..small_shape.offset(x / 2, y / 2) + channels];
            let slot = &mut target[target_shape.offset(x, y)..target_shape.offset(x, y) + channels];
            for (t, &s) in slot.iter_mut().zip(source) {
                let sum = target_quant.dequantize16(*t) + small_quant.dequantize16(s);
                *t = out_quant.quantize16(sum);
            }
        }
    }
}

/// The four heads on one feature map (mapped by `quant`), as in the `f32`
/// pass: the 1x1 convolution quantized with its node's mapping into
/// `work`, then the 3x3 convolution in `f32`.
fn heads<'a>(
    weights: &impl Weights,
    level: &Level,
    feature: &[i16],
    quant: Quant,
    shape: Shape,
    work: &mut [i16],
    out: HeadBuffers<'a>,
) -> Heads<'a> {
    let HeadBuffers {
        cls,
        obj,
        bbox,
        kps,
    } = out;
    for (kind, node, output) in [("cls", level.cls, &mut *cls), ("obj", level.obj, &mut *obj)] {
        // `conv1`: 1x1, 64 -> 1. `conv2`: a full 3x3 convolution over that
        // one channel, padding 1. Then the sigmoid.
        let mid_quant = quant_of(weights, node);
        let mid = quant::conv2d_16(
            feature,
            quant,
            shape,
            &tensors(weights, head_name(kind, level, "conv1").as_str()),
            1,
            1,
            0,
            Activation::None,
            Output::I16(&mut *work, mid_quant),
        );
        quant::conv2d_16(
            view(work, mid),
            mid_quant,
            mid,
            &tensors(weights, head_name(kind, level, "conv2").as_str()),
            3,
            1,
            1,
            Activation::None,
            Output::F32(output),
        );
        sigmoid(output);
    }
    for (kind, node, output) in [
        ("bbox", level.bbox, &mut *bbox),
        ("kps", level.kps, &mut *kps),
    ] {
        // `conv1`: 1x1, 64 -> 4 or 10. `conv2`: depthwise 3x3, padding 1.
        // No sigmoid: these are offsets.
        let mid_quant = quant_of(weights, node);
        let mid = quant::conv2d_16(
            feature,
            quant,
            shape,
            &tensors(weights, head_name(kind, level, "conv1").as_str()),
            1,
            1,
            0,
            Activation::None,
            Output::I16(&mut *work, mid_quant),
        );
        quant::depthwise_16(
            view(work, mid),
            mid_quant,
            mid,
            &tensors(weights, head_name(kind, level, "conv2").as_str()),
            3,
            1,
            1,
            Activation::None,
            Output::F32(output),
        );
    }
    Heads {
        stride: level.stride,
        map: Shape::new(shape.height, shape.width, 1),
        cls,
        obj,
        bbox,
        kps,
    }
}

/// Run the detector on `input`, a [`INPUT_SHAPE`] image as `i8` under
/// [`INPUT_QUANT`] (see `detect::detector_input_i8`), and return the heads
/// for stride 8, 16 and 32, the same type as the `f32` pass returns. The
/// heads point into the scratch's `f32` part.
///
/// # Panics
///
/// When `input` is not [`INPUT_SHAPE`].
pub fn forward<'a>(weights: &impl Weights, input: &[i8], scratch: Scratch<'a>) -> [Heads<'a>; 3] {
    let mut tracer = Tracer {
        trace: None::<fn(&str, Shape, &[f32])>,
    };
    run(weights, input, scratch, &mut tracer)
}

/// [`forward`], calling `trace(node, shape, values)` with the same nodes
/// as the `f32` pass's `forward_traced`, in the same order, each
/// dequantized into the scratch's `f32` work buffer so a test can compare
/// the two passes stage by stage.
///
/// # Panics
///
/// When `input` is not [`INPUT_SHAPE`].
pub fn forward_traced<'a>(
    weights: &impl Weights,
    input: &[i8],
    scratch: Scratch<'a>,
    trace: impl FnMut(&str, Shape, &[f32]),
) -> [Heads<'a>; 3] {
    let mut tracer = Tracer { trace: Some(trace) };
    run(weights, input, scratch, &mut tracer)
}

/// The forward pass over split buffers. The node comments follow the
/// `f32` pass.
fn run<'a>(
    weights: &impl Weights,
    input: &[i8],
    scratch: Scratch<'a>,
    tracer: &mut Tracer<impl FnMut(&str, Shape, &[f32])>,
) -> [Heads<'a>; 3] {
    assert_eq!(
        input.len(),
        INPUT_SHAPE.len(),
        "yunet input: {} values, need {}",
        input.len(),
        INPUT_SHAPE.len()
    );
    let Scratch {
        work_a,
        work_b,
        map_8,
        map_16,
        map_32,
        mid,
        input_wide,
        work_f32,
        heads: [out_8, out_16, out_32],
    } = scratch;

    // Stem, on the 32x48 map.
    // Conv_0 + Relu_1: 3x3, stride 2, padding 1, 3 -> 16 channels, from
    // the 8-bit image to the first 16-bit map. The image is widened to
    // eight `i16` channels with its zero point removed, and the weights
    // padded to match, so the vector unit runs it; the padding channels
    // are zero on both sides, so the sums are those of the 3-channel
    // convolution.
    let quant = quant_of(weights, "Relu_1");
    let shape = {
        const TAPS: usize = 9;
        let narrow = tensors(weights, "Conv_0");
        let filters = narrow.bias.len();
        assert_eq!(
            narrow.data.len(),
            filters * TAPS * INPUT_SHAPE.channels,
            "Conv_0 weight"
        );
        assert_eq!(filters, STEM_FILTERS, "Conv_0 filters");
        let mut padded = simd::Aligned([0i8; STEM_FILTERS * TAPS * INPUT_PADDED]);
        let per_output = TAPS * INPUT_SHAPE.channels;
        for (index, wide) in padded.0.chunks_exact_mut(INPUT_PADDED).enumerate() {
            let (o, tap) = (index / TAPS, index % TAPS);
            for (c, w) in wide[..INPUT_SHAPE.channels].iter_mut().enumerate() {
                *w = narrow.at(o, tap * INPUT_SHAPE.channels + c, per_output);
            }
        }
        let weight = QWeight {
            data: &padded.0,
            scales: narrow.scales,
            bias: narrow.bias,
            packed: false,
        };
        input_wide.fill(0);
        for (wide, narrow) in input_wide
            .chunks_exact_mut(INPUT_PADDED)
            .zip(input.chunks_exact(INPUT_SHAPE.channels))
        {
            for (w, &n) in wide.iter_mut().zip(narrow) {
                *w = (i32::from(n) - INPUT_QUANT.zero_point) as i16;
            }
        }
        quant::conv2d_16(
            input_wide,
            Quant {
                scale: INPUT_QUANT.scale,
                zero_point: 0,
            },
            Shape::new(INPUT_SHAPE.height, INPUT_SHAPE.width, INPUT_PADDED),
            &weight,
            3,
            2,
            1,
            Activation::Relu,
            Output::I16(work_a, quant),
        )
    };
    tracer.emit(work_f32, "Relu_1", shape, work_a, quant);
    // Conv_2 (1x1, 16 -> 16), Conv_3 (depthwise), Relu_4.
    let (shape, quant) = unit(weights, &UNITS[0], work_a, quant, shape, mid);
    tracer.emit(work_f32, "Relu_4", shape, work_a, quant);

    // Stage 1, on the 16x24 map.
    // MaxPool_5: the mapping does not change.
    let shape = max_pool_2x2_i16(view(work_a, shape), shape, work_b);
    tracer.emit(work_f32, "MaxPool_5", shape, work_b, quant);
    // Conv_6 (1x1, 16 -> 16), Conv_7, Relu_8.
    let (shape, quant) = unit(weights, &UNITS[1], work_b, quant, shape, mid);
    tracer.emit(work_f32, "Relu_8", shape, work_b, quant);
    // Conv_9 (1x1, 16 -> 32), Conv_10, Relu_11.
    let (shape, quant) = unit(weights, &UNITS[2], work_b, quant, shape, mid);
    tracer.emit(work_f32, "Relu_11", shape, work_b, quant);
    // Conv_12 (1x1, 32 -> 32), Conv_13, Relu_14.
    let (shape, quant) = unit(weights, &UNITS[3], work_b, quant, shape, mid);
    tracer.emit(work_f32, "Relu_14", shape, work_b, quant);
    // Conv_15 (1x1, 32 -> 64), Conv_16, Relu_17.
    let (shape, quant) = unit(weights, &UNITS[4], work_b, quant, shape, mid);
    tracer.emit(work_f32, "Relu_17", shape, work_b, quant);

    // Stage 2, on the 8x12 map. From here on every unit is 64 -> 64.
    // MaxPool_18.
    let shape = max_pool_2x2_i16(view(work_b, shape), shape, map_8);
    tracer.emit(work_f32, "MaxPool_18", shape, map_8, quant);
    // Conv_19, Conv_20, Relu_21.
    let (shape, quant) = unit(weights, &UNITS[5], map_8, quant, shape, mid);
    tracer.emit(work_f32, "Relu_21", shape, map_8, quant);
    // Conv_22, Conv_23, Relu_24: the backbone's 8x map, kept for Add_50.
    let (shape_8, quant_8) = unit(weights, &UNITS[6], map_8, quant, shape, mid);
    tracer.emit(work_f32, "Relu_24", shape_8, map_8, quant_8);

    // Stage 3, on the 4x6 map.
    // MaxPool_25.
    let shape = max_pool_2x2_i16(view(map_8, shape_8), shape_8, map_16);
    tracer.emit(work_f32, "MaxPool_25", shape, map_16, quant_8);
    // Conv_26, Conv_27, Relu_28.
    let (shape, quant) = unit(weights, &UNITS[7], map_16, quant_8, shape, mid);
    tracer.emit(work_f32, "Relu_28", shape, map_16, quant);
    // Conv_29, Conv_30, Relu_31: the backbone's 16x map, kept for Add_44.
    let (shape_16, quant_16) = unit(weights, &UNITS[8], map_16, quant, shape, mid);
    tracer.emit(work_f32, "Relu_31", shape_16, map_16, quant_16);

    // Stage 4, on the 2x3 map.
    // MaxPool_32.
    let shape = max_pool_2x2_i16(view(map_16, shape_16), shape_16, map_32);
    tracer.emit(work_f32, "MaxPool_32", shape, map_32, quant_16);
    // Conv_33, Conv_34, Relu_35.
    let (shape, quant) = unit(weights, &UNITS[9], map_32, quant_16, shape, mid);
    tracer.emit(work_f32, "Relu_35", shape, map_32, quant);
    // Conv_36, Conv_37, Relu_38.
    let (shape, quant) = unit(weights, &UNITS[10], map_32, quant, shape, mid);
    tracer.emit(work_f32, "Relu_38", shape, map_32, quant);

    // Neck.
    // Conv_39 (lateral 2), Conv_40, Relu_41: the stride-32 feature map.
    let (shape_32, quant_32) = unit(weights, &UNITS[11], map_32, quant, shape, mid);
    tracer.emit(work_f32, "Relu_41", shape_32, map_32, quant_32);
    // Resize_43 (nearest, 2x) + Add_44: onto the backbone's 16x map, which
    // takes the sum's mapping.
    let quant_16 = {
        let sum = quant_of(weights, "Add_44");
        upsample_2x_add_i16(
            view(map_32, shape_32),
            quant_32,
            shape_32,
            &mut map_16[..shape_16.len()],
            quant_16,
            shape_16,
            sum,
        );
        sum
    };
    tracer.emit(work_f32, "Add_44", shape_16, map_16, quant_16);
    // Conv_45 (lateral 1), Conv_46, Relu_47: the stride-16 feature map.
    let (shape_16, quant_16) = unit(weights, &UNITS[12], map_16, quant_16, shape_16, mid);
    tracer.emit(work_f32, "Relu_47", shape_16, map_16, quant_16);
    // Resize_49 (nearest, 2x) + Add_50: onto the backbone's 8x map.
    let quant_8 = {
        let sum = quant_of(weights, "Add_50");
        upsample_2x_add_i16(
            view(map_16, shape_16),
            quant_16,
            shape_16,
            &mut map_8[..shape_8.len()],
            quant_8,
            shape_8,
            sum,
        );
        sum
    };
    tracer.emit(work_f32, "Add_50", shape_8, map_8, quant_8);
    // Conv_51 (lateral 0), Conv_52, Relu_53: the stride-8 feature map.
    let (shape_8, quant_8) = unit(weights, &UNITS[13], map_8, quant_8, shape_8, mid);
    tracer.emit(work_f32, "Relu_53", shape_8, map_8, quant_8);

    // Heads: Conv_54 to Conv_77, and the sigmoids of `cls` and `obj`.
    [
        heads(
            weights,
            &LEVELS[0],
            view(map_8, shape_8),
            quant_8,
            shape_8,
            mid,
            out_8,
        ),
        heads(
            weights,
            &LEVELS[1],
            view(map_16, shape_16),
            quant_16,
            shape_16,
            mid,
            out_16,
        ),
        heads(
            weights,
            &LEVELS[2],
            view(map_32, shape_32),
            quant_32,
            shape_32,
            mid,
            out_32,
        ),
    ]
}
