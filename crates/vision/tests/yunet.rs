//! The YuNet forward pass against the golden vectors tract produced from
//! the ONNX model (see `tests/fixtures/README.md`).
//!
//! Every Relu, MaxPool and Add output of the graph is compared with the
//! model's, so a wrong reading of the graph shows up at the first node
//! that differs, not just at the heads.

mod common;

use common::{Tensors, assert_close};
use hack_and_hike_vision::nn::{
    Shape,
    yunet::{BBOX_LEN, INPUT_SHAPE, KPS_LEN, SCRATCH_LEN, STRIDES, forward, forward_traced},
};

/// The weights of YuNet, as exported by facekit.
const WEIGHTS: &[u8] = include_bytes!("fixtures/yunet.f32.fkb");
/// Reference outputs of YuNet on `face_320x240.jpg`.
const GOLDEN: &[u8] = include_bytes!("fixtures/yunet.golden.fkb");
/// The absolute tolerance for every comparison.
const ATOL: f32 = 1e-4;
/// The relative tolerance (times the largest golden value) for every
/// comparison.
const RTOL: f32 = 1e-4;
/// The nodes the forward pass traces, in graph order: every Relu, MaxPool
/// and Add. The golden file holds each of them.
const TRACED_NODES: [&str; 21] = [
    "Relu_1",
    "Relu_4",
    "MaxPool_5",
    "Relu_8",
    "Relu_11",
    "Relu_14",
    "Relu_17",
    "MaxPool_18",
    "Relu_21",
    "Relu_24",
    "MaxPool_25",
    "Relu_28",
    "Relu_31",
    "MaxPool_32",
    "Relu_35",
    "Relu_38",
    "Relu_41",
    "Add_44",
    "Relu_47",
    "Add_50",
    "Relu_53",
];

/// The golden input, converted to the kernels' layout.
fn golden_input(golden: &Tensors) -> Vec<f32> {
    let (shape, input) = golden.activation_nchw("input");
    assert_eq!(shape, INPUT_SHAPE);
    input
}

#[test]
fn every_stage_matches_the_golden_vectors() {
    let weights = Tensors::load(WEIGHTS);
    let golden = Tensors::load(GOLDEN);
    let input = golden_input(&golden);
    let mut scratch = vec![0.0f32; SCRATCH_LEN];

    let mut traced = Vec::new();
    let heads = forward_traced(&weights, &input, &mut scratch, |node, shape, values| {
        let (expected_shape, expected) = golden.activation_nchw(node);
        assert_eq!(shape, expected_shape, "{node} shape");
        assert_close(node, values, &expected, ATOL, RTOL);
        traced.push(node.to_string());
    });
    assert_eq!(traced, TRACED_NODES, "traced nodes");

    for (heads, stride) in heads.iter().zip(STRIDES) {
        assert_eq!(heads.stride, stride);
        assert_eq!(heads.map, Shape::new(64 / stride, 96 / stride, 1));
        let anchors = heads.map.pixels();
        assert_eq!((heads.cls.len(), heads.obj.len()), (anchors, anchors));
        assert_eq!(
            (heads.bbox.len(), heads.kps.len()),
            (anchors * BBOX_LEN, anchors * KPS_LEN)
        );
        for (name, actual) in [
            ("cls", heads.cls),
            ("obj", heads.obj),
            ("bbox", heads.bbox),
            ("kps", heads.kps),
        ] {
            let name = format!("{name}_{stride}");
            assert_close(&name, actual, golden.flat(&name), ATOL, RTOL);
        }
    }
}

#[test]
fn finds_the_face_where_the_photo_has_it() {
    let weights = Tensors::load(WEIGHTS);
    let golden = Tensors::load(GOLDEN);
    let input = golden_input(&golden);
    let mut scratch = vec![0.0f32; SCRATCH_LEN];

    let [stride_8, _, _] = forward(&weights, &input, &mut scratch);
    // The face fills the frame height of the 80x60 photo in the top-left
    // corner of the input, so its centre is near (36, 36): the stride-8
    // anchor in row 4, column 4. The score is the square root of
    // `cls * obj`, as in OpenCV.
    let (best, score) = stride_8
        .cls
        .iter()
        .zip(stride_8.obj)
        .map(|(cls, obj)| (cls * obj).max(0.0).sqrt())
        .enumerate()
        .fold((0, 0.0f32), |best, (index, score)| {
            if score > best.1 { (index, score) } else { best }
        });
    let (row, column) = (best / stride_8.map.width, best % stride_8.map.width);
    println!("best stride-8 anchor: row {row}, column {column}, score {score:.3}");
    assert!(score > 0.8, "best face score {score}");
    assert_eq!((row, column), (4, 4), "face position");
}
