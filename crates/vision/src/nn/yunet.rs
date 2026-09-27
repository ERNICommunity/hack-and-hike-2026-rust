//! YuNet, the face detector: the `f32` reference forward pass.
//!
//! YuNet is a small convolutional network. It takes a 96x64 BGR image
//! (values 0..255) and predicts, for every cell of three grids laid over the
//! image (one cell per 8, 16 and 32 pixels), whether a face is centred
//! there, its box and its five landmarks. The firmware puts the 80x60 camera
//! image in the top-left corner of a black 96x64 frame.
//!
//! The graph, node by node, is in `docs/face_id/inventory_yunet_2023mar.md`.
//! In short:
//!
//! - A stem: one 3x3 convolution with stride 2, then the first "unit".
//! - A unit is a 1x1 convolution (no activation), a depthwise 3x3
//!   convolution and a Relu. The backbone is nine more units with a 2x2 max
//!   pooling between stages, so the image gets 2x, 4x, 8x, 16x and 32x
//!   smaller.
//! - A neck: a unit on the 32x map, enlarged 2x and added to the 16x map, a
//!   unit on that, enlarged and added to the 8x map, and a unit on that. The
//!   three unit outputs are the feature maps of the heads.
//! - Four heads on each feature map: `cls` and `obj` (1x1 then 3x3
//!   convolution, sigmoid), `bbox` and `kps` (1x1 then depthwise 3x3, no
//!   sigmoid).
//!
//! Every activation is channels-last (see [`super`]), and the head outputs
//! are one row of values per grid cell in row-major order: exactly what the
//! ONNX model's final `Transpose` and `Reshape` produce, so they compare
//! directly with the model's outputs.
//!
//! Nothing allocates: the caller gives a scratch slice of [`SCRATCH_LEN`]
//! values (240 KiB) that the forward pass splits into its buffers. The
//! firmware allocates it once in PSRAM. The weights come from a
//! [`Weights`] source by the names facekit exported them under: a
//! convolution that has a PyTorch name keeps it (`backbone.model1.conv1.conv1`),
//! one that has none is named after its ONNX node (`Conv_7`).
//!
//! The integer version of the same pass, the one the firmware runs, is in
//! [`int8`]. It needs a mapping to `i8` for every activation tensor; the
//! calibration tool (`facekit quantize`) gets the values it measures from
//! [`forward_probed`].

pub mod int8;

use super::{
    Activation, Shape, Weights, conv2d, depthwise, max_pool_2x2, quant::Granularity, sigmoid,
    upsample_2x_add,
};
use crate::blob::NAME_LEN;

/// Rows of the input image.
pub const INPUT_HEIGHT: usize = 64;
/// Columns of the input image.
pub const INPUT_WIDTH: usize = 96;
/// Channels of the input image: blue, green, red.
pub const INPUT_CHANNELS: usize = 3;
/// The shape of the input tensor.
pub const INPUT_SHAPE: Shape = Shape::new(INPUT_HEIGHT, INPUT_WIDTH, INPUT_CHANNELS);
/// The strides of the three heads, in the order [`forward`] returns them.
pub const STRIDES: [usize; 3] = [8, 16, 32];
/// Values per anchor in a `bbox` output: centre x, centre y, width, height,
/// as offsets that the decoder scales by the stride.
pub const BBOX_LEN: usize = 4;
/// Values per anchor in a `kps` output: five landmarks as `(x, y)` pairs.
pub const KPS_LEN: usize = 10;

/// The map after the stem's stride-2 convolution.
const STEM: Shape = Shape::new(INPUT_HEIGHT / 2, INPUT_WIDTH / 2, 16);
/// The widest map of the first pooled stage (its last unit has 64 channels).
const STAGE1: Shape = Shape::new(INPUT_HEIGHT / 4, INPUT_WIDTH / 4, 64);
/// The feature map of the stride-8 head.
const MAP_8: Shape = Shape::new(INPUT_HEIGHT / 8, INPUT_WIDTH / 8, 64);
/// The feature map of the stride-16 head.
const MAP_16: Shape = Shape::new(INPUT_HEIGHT / 16, INPUT_WIDTH / 16, 64);
/// The feature map of the stride-32 head.
const MAP_32: Shape = Shape::new(INPUT_HEIGHT / 32, INPUT_WIDTH / 32, 64);
/// The largest activation before the 8x stage: a work buffer holds one.
const WORK_LEN: usize = if STEM.len() > STAGE1.len() {
    STEM.len()
} else {
    STAGE1.len()
};
/// The largest 1x1 result inside a head: the stride-8 `kps` head's.
const HEAD_WORK_LEN: usize = MAP_8.pixels() * KPS_LEN;
/// Values the four heads of one map produce per anchor.
const HEAD_VALUES: usize = 1 + 1 + BBOX_LEN + KPS_LEN;
/// Anchors of all three heads together.
const ANCHORS: usize = MAP_8.pixels() + MAP_16.pixels() + MAP_32.pixels();
/// The length of the scratch slice [`forward`] needs.
pub const SCRATCH_LEN: usize = 2 * WORK_LEN
    + MAP_8.len()
    + MAP_16.len()
    + MAP_32.len()
    + HEAD_WORK_LEN
    + ANCHORS * HEAD_VALUES;

/// The names of one unit of the backbone or the neck.
struct Unit {
    /// The 1x1 convolution's weight tensor: its PyTorch name.
    pointwise: &'static str,
    /// The ONNX node of the 1x1 convolution, which names its output.
    pointwise_node: &'static str,
    /// The depthwise convolution, named after its ONNX node.
    depthwise_node: &'static str,
    /// The Relu node after it, which names the unit's output.
    relu_node: &'static str,
}

impl Unit {
    /// A unit's names, in graph order.
    const fn new(
        pointwise: &'static str,
        pointwise_node: &'static str,
        depthwise_node: &'static str,
        relu_node: &'static str,
    ) -> Self {
        Self {
            pointwise,
            pointwise_node,
            depthwise_node,
            relu_node,
        }
    }
}

/// The fourteen units of the graph, in graph order: eleven in the backbone
/// (one after the stem, two per pooled stage), then the three of the neck
/// on the 32x, 16x and 8x maps.
const UNITS: [Unit; 14] = [
    Unit::new("backbone.model0.conv2.conv1", "Conv_2", "Conv_3", "Relu_4"),
    Unit::new("backbone.model1.conv1.conv1", "Conv_6", "Conv_7", "Relu_8"),
    Unit::new(
        "backbone.model1.conv2.conv1",
        "Conv_9",
        "Conv_10",
        "Relu_11",
    ),
    Unit::new(
        "backbone.model2.conv1.conv1",
        "Conv_12",
        "Conv_13",
        "Relu_14",
    ),
    Unit::new(
        "backbone.model2.conv2.conv1",
        "Conv_15",
        "Conv_16",
        "Relu_17",
    ),
    Unit::new(
        "backbone.model3.conv1.conv1",
        "Conv_19",
        "Conv_20",
        "Relu_21",
    ),
    Unit::new(
        "backbone.model3.conv2.conv1",
        "Conv_22",
        "Conv_23",
        "Relu_24",
    ),
    Unit::new(
        "backbone.model4.conv1.conv1",
        "Conv_26",
        "Conv_27",
        "Relu_28",
    ),
    Unit::new(
        "backbone.model4.conv2.conv1",
        "Conv_29",
        "Conv_30",
        "Relu_31",
    ),
    Unit::new(
        "backbone.model5.conv1.conv1",
        "Conv_33",
        "Conv_34",
        "Relu_35",
    ),
    Unit::new(
        "backbone.model5.conv2.conv1",
        "Conv_36",
        "Conv_37",
        "Relu_38",
    ),
    Unit::new(
        "neck.lateral_convs.2.conv1",
        "Conv_39",
        "Conv_40",
        "Relu_41",
    ),
    Unit::new(
        "neck.lateral_convs.1.conv1",
        "Conv_45",
        "Conv_46",
        "Relu_47",
    ),
    Unit::new(
        "neck.lateral_convs.0.conv1",
        "Conv_51",
        "Conv_52",
        "Relu_53",
    ),
];

/// One level of the heads: the feature map of one stride.
struct Level {
    /// The level's index in the weight names: `bbox_head.multi_level_cls.<name>`.
    name: &'static str,
    /// Input pixels per anchor.
    stride: usize,
    /// The ONNX node of the `cls` head's 1x1 convolution, which names its
    /// output.
    cls: &'static str,
    /// The same for the `obj` head.
    obj: &'static str,
    /// The same for the `bbox` head.
    bbox: &'static str,
    /// The same for the `kps` head.
    kps: &'static str,
}

/// The three levels, in the order [`forward`] returns them.
const LEVELS: [Level; 3] = [
    Level {
        name: "0",
        stride: STRIDES[0],
        cls: "Conv_54",
        obj: "Conv_66",
        bbox: "Conv_60",
        kps: "Conv_72",
    },
    Level {
        name: "1",
        stride: STRIDES[1],
        cls: "Conv_56",
        obj: "Conv_68",
        bbox: "Conv_62",
        kps: "Conv_74",
    },
    Level {
        name: "2",
        stride: STRIDES[2],
        cls: "Conv_58",
        obj: "Conv_70",
        bbox: "Conv_64",
        kps: "Conv_76",
    },
];

/// The raw outputs of the four heads on one grid.
///
/// The grid has `map.height` rows and `map.width` columns of anchors, one
/// per `stride` pixels of the input; anchor `(column, row)` is index
/// `row * map.width + column`. `cls` and `obj` hold one value per anchor,
/// after the sigmoid; `bbox` holds [`BBOX_LEN`] and `kps` [`KPS_LEN`] values
/// per anchor, raw. The decoder (a later step) turns them into boxes.
#[derive(Debug)]
pub struct Heads<'a> {
    /// Input pixels per anchor: 8, 16 or 32.
    pub stride: usize,
    /// Rows and columns of the anchor grid; `channels` is 1.
    pub map: Shape,
    /// Face probability per anchor.
    pub cls: &'a [f32],
    /// Objectness per anchor.
    pub obj: &'a [f32],
    /// Box offsets per anchor.
    pub bbox: &'a [f32],
    /// Landmark offsets per anchor.
    pub kps: &'a [f32],
}

/// The output buffers of the four heads on one grid.
struct HeadBuffers<'a> {
    /// One value per anchor.
    cls: &'a mut [f32],
    /// One value per anchor.
    obj: &'a mut [f32],
    /// [`BBOX_LEN`] values per anchor.
    bbox: &'a mut [f32],
    /// [`KPS_LEN`] values per anchor.
    kps: &'a mut [f32],
}

impl<'a> HeadBuffers<'a> {
    /// Cut the head buffers of the three levels off the front of `rest`.
    fn take_all(rest: &mut &'a mut [f32]) -> [Self; 3] {
        [MAP_8, MAP_16, MAP_32].map(|map| Self {
            cls: take(rest, map.pixels()),
            obj: take(rest, map.pixels()),
            bbox: take(rest, map.pixels() * BBOX_LEN),
            kps: take(rest, map.pixels() * KPS_LEN),
        })
    }
}

/// The scratch memory of one forward pass, split into its buffers.
///
/// Make one with [`Scratch::new`] over a slice of at least [`SCRATCH_LEN`]
/// values. The forward pass borrows it for the duration of the pass and the
/// returned [`Heads`] point into it.
pub struct Scratch<'a> {
    /// A work buffer for the stem and the first stage.
    work_a: &'a mut [f32],
    /// The other work buffer: units alternate between the two.
    work_b: &'a mut [f32],
    /// The 8x map: the backbone's, then the neck's sum, then the feature map.
    map_8: &'a mut [f32],
    /// The 16x map, likewise.
    map_16: &'a mut [f32],
    /// The 32x map: the backbone's, then the neck's feature map.
    map_32: &'a mut [f32],
    /// The 1x1 result inside a head.
    head_work: &'a mut [f32],
    /// The outputs of the heads, per stride.
    heads: [HeadBuffers<'a>; 3],
}

impl<'a> Scratch<'a> {
    /// Split `buffer` into the buffers of a forward pass.
    ///
    /// # Panics
    ///
    /// When `buffer` is shorter than [`SCRATCH_LEN`].
    pub fn new(buffer: &'a mut [f32]) -> Self {
        assert!(
            buffer.len() >= SCRATCH_LEN,
            "yunet scratch: {} values, need {SCRATCH_LEN}",
            buffer.len()
        );
        let mut rest = buffer;
        let work_a = take(&mut rest, WORK_LEN);
        let work_b = take(&mut rest, WORK_LEN);
        let map_8 = take(&mut rest, MAP_8.len());
        let map_16 = take(&mut rest, MAP_16.len());
        let map_32 = take(&mut rest, MAP_32.len());
        let head_work = take(&mut rest, HEAD_WORK_LEN);
        let heads = HeadBuffers::take_all(&mut rest);
        Self {
            work_a,
            work_b,
            map_8,
            map_16,
            map_32,
            head_work,
            heads,
        }
    }
}

/// Cut the first `len` values off the front of `rest` and return them.
fn take<'a, T>(rest: &mut &'a mut [T], len: usize) -> &'a mut [T] {
    let (front, back) = core::mem::take(rest).split_at_mut(len);
    *rest = back;
    front
}

/// A tensor name joined from pieces without allocating. FKB1 names are at
/// most [`NAME_LEN`] bytes, so a fixed buffer is enough.
struct Name {
    /// The bytes of the name.
    bytes: [u8; NAME_LEN],
    /// How many of them are used.
    len: usize,
}

impl Name {
    /// The concatenation of `parts`.
    ///
    /// # Panics
    ///
    /// When the result is longer than [`NAME_LEN`].
    fn join(parts: &[&str]) -> Self {
        let mut name = Self {
            bytes: [0; NAME_LEN],
            len: 0,
        };
        for part in parts {
            let end = name.len + part.len();
            name.bytes[name.len..end].copy_from_slice(part.as_bytes());
            name.len = end;
        }
        name
    }

    /// The name as text.
    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.len]).expect("joined from str parts")
    }
}

/// The weight and bias of the convolution called `prefix`.
fn tensors<'w>(weights: &'w impl Weights, prefix: &str) -> (&'w [f32], &'w [f32]) {
    (
        weights.get(Name::join(&[prefix, ".weight"]).as_str()),
        weights.get(Name::join(&[prefix, ".bias"]).as_str()),
    )
}

/// The first `shape.len()` values of `buffer`: the activation it holds.
fn view(buffer: &[f32], shape: Shape) -> &[f32] {
    &buffer[..shape.len()]
}

/// Give a traced node to both the trace and the probe.
fn tap(
    trace: &mut impl FnMut(&str, Shape, &[f32]),
    probe: &mut impl FnMut(&str, Granularity, &[f32]),
    node: &str,
    shape: Shape,
    values: &[f32],
) {
    trace(node, shape, values);
    probe(node, Granularity::Wide, values);
}

/// One unit of the backbone or the neck: the 1x1 convolution with no
/// activation, then the depthwise 3x3 convolution with padding 1 and Relu.
///
/// `data` holds the input of `shape` and receives the output; `work` holds
/// the 1x1 result in between, which goes to `probe` under its node name.
/// Returns the output shape: the same pixels with the unit's channel count.
fn unit(
    weights: &impl Weights,
    unit: &Unit,
    data: &mut [f32],
    shape: Shape,
    work: &mut [f32],
    probe: &mut impl FnMut(&str, Granularity, &[f32]),
) -> Shape {
    let (weight, bias) = tensors(weights, unit.pointwise);
    let mid = conv2d(
        view(data, shape),
        shape,
        weight,
        bias,
        1,
        1,
        0,
        Activation::None,
        work,
    );
    probe(unit.pointwise_node, Granularity::Wide, view(work, mid));
    let (weight, bias) = tensors(weights, unit.depthwise_node);
    depthwise(
        view(work, mid),
        mid,
        weight,
        bias,
        3,
        1,
        1,
        Activation::Relu,
        data,
    )
}

/// The four heads on one feature map. Each head's 1x1 result goes to
/// `probe` under its node name.
fn heads<'a>(
    weights: &impl Weights,
    level: &Level,
    feature: &[f32],
    shape: Shape,
    work: &mut [f32],
    out: HeadBuffers<'a>,
    probe: &mut impl FnMut(&str, Granularity, &[f32]),
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
        let (weight, bias) = tensors(
            weights,
            Name::join(&["bbox_head.multi_level_", kind, ".", level.name, ".conv1"]).as_str(),
        );
        let mid = conv2d(
            feature,
            shape,
            weight,
            bias,
            1,
            1,
            0,
            Activation::None,
            work,
        );
        probe(node, Granularity::Wide, view(work, mid));
        let (weight, bias) = tensors(
            weights,
            Name::join(&["bbox_head.multi_level_", kind, ".", level.name, ".conv2"]).as_str(),
        );
        conv2d(
            view(work, mid),
            mid,
            weight,
            bias,
            3,
            1,
            1,
            Activation::None,
            output,
        );
        sigmoid(output);
    }
    for (kind, node, output) in [
        ("bbox", level.bbox, &mut *bbox),
        ("kps", level.kps, &mut *kps),
    ] {
        // `conv1`: 1x1, 64 -> 4 or 10. `conv2`: depthwise 3x3, padding 1.
        // No sigmoid: these are offsets.
        let (weight, bias) = tensors(
            weights,
            Name::join(&["bbox_head.multi_level_", kind, ".", level.name, ".conv1"]).as_str(),
        );
        let mid = conv2d(
            feature,
            shape,
            weight,
            bias,
            1,
            1,
            0,
            Activation::None,
            work,
        );
        probe(node, Granularity::Wide, view(work, mid));
        let (weight, bias) = tensors(
            weights,
            Name::join(&["bbox_head.multi_level_", kind, ".", level.name, ".conv2"]).as_str(),
        );
        depthwise(
            view(work, mid),
            mid,
            weight,
            bias,
            3,
            1,
            1,
            Activation::None,
            output,
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

/// Run the detector on `input`, a [`INPUT_SHAPE`] image (BGR, 0..255), and
/// return the heads for stride 8, 16 and 32. `scratch` is at least
/// [`SCRATCH_LEN`] values; the heads point into it.
///
/// # Panics
///
/// When `input` is not [`INPUT_SHAPE`] or `scratch` is too short.
pub fn forward<'a>(
    weights: &impl Weights,
    input: &[f32],
    scratch: &'a mut [f32],
) -> [Heads<'a>; 3] {
    forward_traced(weights, input, scratch, |_, _, _| {})
}

/// [`forward`], calling `trace(node, shape, values)` with the output of
/// every Relu, MaxPool and Add node of the ONNX graph, in graph order, so
/// a test can compare each stage with the model's own numbers. The two
/// Resize nodes are fused into the Adds that follow them and are not
/// traced.
///
/// # Panics
///
/// When `input` is not [`INPUT_SHAPE`] or `scratch` is too short.
pub fn forward_traced<'a>(
    weights: &impl Weights,
    input: &[f32],
    scratch: &'a mut [f32],
    trace: impl FnMut(&str, Shape, &[f32]),
) -> [Heads<'a>; 3] {
    forward_probed(weights, input, scratch, trace, |_, _, _| {})
}

/// [`forward_traced`], also calling `probe(node, granularity, values)`
/// with every activation tensor the integer pass ([`int8`]) keeps as
/// `i8`: the traced nodes, the 1x1 convolution output inside every unit
/// (`Conv_2`, `Conv_6`, ...) and the 1x1 convolution output inside every
/// head (`Conv_54`, `Conv_56`, ...). Every one of them gets a wide
/// (16-bit, per-tensor) mapping, [`Granularity::Wide`]: the detector is
/// small enough to afford it. The calibration tool records the range of
/// each over many images and stores the mappings as `q.<node>` in the
/// integer weights file.
///
/// # Panics
///
/// When `input` is not [`INPUT_SHAPE`] or `scratch` is too short.
pub fn forward_probed<'a>(
    weights: &impl Weights,
    input: &[f32],
    scratch: &'a mut [f32],
    mut trace: impl FnMut(&str, Shape, &[f32]),
    mut probe: impl FnMut(&str, Granularity, &[f32]),
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
        head_work,
        heads: [out_8, out_16, out_32],
    } = Scratch::new(scratch);

    // Stem, on the 32x48 map.
    // Conv_0 + Relu_1: 3x3, stride 2, padding 1, 3 -> 16 channels.
    let (weight, bias) = tensors(weights, "Conv_0");
    let shape = conv2d(
        input,
        INPUT_SHAPE,
        weight,
        bias,
        3,
        2,
        1,
        Activation::Relu,
        work_a,
    );
    tap(&mut trace, &mut probe, "Relu_1", shape, view(work_a, shape));
    // Conv_2 (1x1, 16 -> 16), Conv_3 (depthwise), Relu_4.
    let shape = unit(weights, &UNITS[0], work_a, shape, work_b, &mut probe);
    tap(&mut trace, &mut probe, "Relu_4", shape, view(work_a, shape));

    // Stage 1, on the 16x24 map.
    // MaxPool_5.
    let shape = max_pool_2x2(view(work_a, shape), shape, work_b);
    tap(
        &mut trace,
        &mut probe,
        "MaxPool_5",
        shape,
        view(work_b, shape),
    );
    // Conv_6 (1x1, 16 -> 16), Conv_7, Relu_8.
    let shape = unit(weights, &UNITS[1], work_b, shape, work_a, &mut probe);
    tap(&mut trace, &mut probe, "Relu_8", shape, view(work_b, shape));
    // Conv_9 (1x1, 16 -> 32), Conv_10, Relu_11.
    let shape = unit(weights, &UNITS[2], work_b, shape, work_a, &mut probe);
    tap(
        &mut trace,
        &mut probe,
        "Relu_11",
        shape,
        view(work_b, shape),
    );
    // Conv_12 (1x1, 32 -> 32), Conv_13, Relu_14.
    let shape = unit(weights, &UNITS[3], work_b, shape, work_a, &mut probe);
    tap(
        &mut trace,
        &mut probe,
        "Relu_14",
        shape,
        view(work_b, shape),
    );
    // Conv_15 (1x1, 32 -> 64), Conv_16, Relu_17.
    let shape = unit(weights, &UNITS[4], work_b, shape, work_a, &mut probe);
    tap(
        &mut trace,
        &mut probe,
        "Relu_17",
        shape,
        view(work_b, shape),
    );

    // Stage 2, on the 8x12 map. From here on every unit is 64 -> 64.
    // MaxPool_18.
    let shape = max_pool_2x2(view(work_b, shape), shape, map_8);
    tap(
        &mut trace,
        &mut probe,
        "MaxPool_18",
        shape,
        view(map_8, shape),
    );
    // Conv_19, Conv_20, Relu_21.
    let shape = unit(weights, &UNITS[5], map_8, shape, work_a, &mut probe);
    tap(&mut trace, &mut probe, "Relu_21", shape, view(map_8, shape));
    // Conv_22, Conv_23, Relu_24: the backbone's 8x map, kept for Add_50.
    let shape_8 = unit(weights, &UNITS[6], map_8, shape, work_a, &mut probe);
    tap(
        &mut trace,
        &mut probe,
        "Relu_24",
        shape_8,
        view(map_8, shape_8),
    );

    // Stage 3, on the 4x6 map.
    // MaxPool_25.
    let shape = max_pool_2x2(view(map_8, shape_8), shape_8, map_16);
    tap(
        &mut trace,
        &mut probe,
        "MaxPool_25",
        shape,
        view(map_16, shape),
    );
    // Conv_26, Conv_27, Relu_28.
    let shape = unit(weights, &UNITS[7], map_16, shape, work_a, &mut probe);
    tap(
        &mut trace,
        &mut probe,
        "Relu_28",
        shape,
        view(map_16, shape),
    );
    // Conv_29, Conv_30, Relu_31: the backbone's 16x map, kept for Add_44.
    let shape_16 = unit(weights, &UNITS[8], map_16, shape, work_a, &mut probe);
    tap(
        &mut trace,
        &mut probe,
        "Relu_31",
        shape_16,
        view(map_16, shape_16),
    );

    // Stage 4, on the 2x3 map.
    // MaxPool_32.
    let shape = max_pool_2x2(view(map_16, shape_16), shape_16, map_32);
    tap(
        &mut trace,
        &mut probe,
        "MaxPool_32",
        shape,
        view(map_32, shape),
    );
    // Conv_33, Conv_34, Relu_35.
    let shape = unit(weights, &UNITS[9], map_32, shape, work_a, &mut probe);
    tap(
        &mut trace,
        &mut probe,
        "Relu_35",
        shape,
        view(map_32, shape),
    );
    // Conv_36, Conv_37, Relu_38.
    let shape = unit(weights, &UNITS[10], map_32, shape, work_a, &mut probe);
    tap(
        &mut trace,
        &mut probe,
        "Relu_38",
        shape,
        view(map_32, shape),
    );

    // Neck.
    // Conv_39 (lateral 2), Conv_40, Relu_41: the stride-32 feature map.
    let shape_32 = unit(weights, &UNITS[11], map_32, shape, work_a, &mut probe);
    tap(
        &mut trace,
        &mut probe,
        "Relu_41",
        shape_32,
        view(map_32, shape_32),
    );
    // Resize_43 (nearest, 2x) + Add_44: onto the backbone's 16x map.
    upsample_2x_add(view(map_32, shape_32), shape_32, map_16, shape_16);
    tap(
        &mut trace,
        &mut probe,
        "Add_44",
        shape_16,
        view(map_16, shape_16),
    );
    // Conv_45 (lateral 1), Conv_46, Relu_47: the stride-16 feature map.
    let shape_16 = unit(weights, &UNITS[12], map_16, shape_16, work_a, &mut probe);
    tap(
        &mut trace,
        &mut probe,
        "Relu_47",
        shape_16,
        view(map_16, shape_16),
    );
    // Resize_49 (nearest, 2x) + Add_50: onto the backbone's 8x map.
    upsample_2x_add(view(map_16, shape_16), shape_16, map_8, shape_8);
    tap(
        &mut trace,
        &mut probe,
        "Add_50",
        shape_8,
        view(map_8, shape_8),
    );
    // Conv_51 (lateral 0), Conv_52, Relu_53: the stride-8 feature map.
    let shape_8 = unit(weights, &UNITS[13], map_8, shape_8, work_a, &mut probe);
    tap(
        &mut trace,
        &mut probe,
        "Relu_53",
        shape_8,
        view(map_8, shape_8),
    );

    // Heads: Conv_54 to Conv_77, and the sigmoids of `cls` and `obj`.
    [
        heads(
            weights,
            &LEVELS[0],
            view(map_8, shape_8),
            shape_8,
            head_work,
            out_8,
            &mut probe,
        ),
        heads(
            weights,
            &LEVELS[1],
            view(map_16, shape_16),
            shape_16,
            head_work,
            out_16,
            &mut probe,
        ),
        heads(
            weights,
            &LEVELS[2],
            view(map_32, shape_32),
            shape_32,
            head_work,
            out_32,
            &mut probe,
        ),
    ]
}
