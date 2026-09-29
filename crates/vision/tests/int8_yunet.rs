//! The integer YuNet against its `f32` twin: every traced stage within
//! the quantization noise, and the same face decoded from both.
//!
//! The integer weights file is built here, in memory, from the `f32`
//! fixture the way `facekit quantize` builds it: weights quantized per
//! output channel, and 16-bit activation mappings from the ranges the
//! `f32` pass's probes report on the golden input. One image and plain
//! min/max ranges are enough for a test; the tool calibrates on many
//! images with better-chosen ranges.

mod common;

use std::collections::{HashMap, HashSet};

use common::Tensors;
use hack_and_hike_vision::{
    detect::{
        DEFAULT_NMS_THRESHOLD, DEFAULT_SCORE_THRESHOLD, Face, decode, detector_input,
        detector_input_i8,
    },
    image::RgbImage,
    nn::{
        Shape, Weights,
        lanes::GroupPlan,
        pack,
        quant::{self, Granularity, Quant},
        yunet::{self, INPUT_SHAPE, int8},
    },
};

/// The weights of YuNet, as exported by facekit.
const WEIGHTS: &[u8] = include_bytes!("fixtures/yunet.f32.fkb");
/// Reference outputs of YuNet on `face_320x240.jpg`.
const GOLDEN: &[u8] = include_bytes!("fixtures/yunet.golden.fkb");
/// The least signal-to-noise ratio every traced stage must reach. With
/// 16-bit activations everywhere, only the 8-bit weights add noise worth
/// mentioning, and every stage stays well above this. The real criterion
/// is the decoded face, checked by the next test.
const MIN_SNR_DB: f32 = 30.0;
/// The largest coordinate difference allowed between the faces the two
/// passes decode, in detector pixels.
const MAX_FACE_DIFFERENCE: f32 = 0.5;
/// The largest score difference allowed between them.
const MAX_SCORE_DIFFERENCE: f32 = 0.005;

/// The range of every value of a probed tensor: its granularity and one
/// `(min, max)` per mapping, so one for a wide or per-tensor probe and
/// one per channel otherwise. The detector only probes wide tensors; the
/// other kinds are handled so the quantizer mirrors facekit's.
type Ranges = HashMap<String, (Granularity, Vec<(f32, f32)>)>;

/// An integer weights file built in memory: `f32` tensors (biases,
/// scales, activation mappings) and `i8` tensors (weights), by name. The
/// weights with a multiple of eight output channels are grouped by eight
/// (`nn::pack`), as the board groups them in its copy of the file.
struct Quantized {
    /// The `f32` tensors.
    f32s: HashMap<String, Vec<f32>>,
    /// The `i8` tensors.
    i8s: HashMap<String, Vec<i8>>,
    /// The names of the grouped `i8` tensors.
    packed: HashSet<String>,
}

impl Weights for Quantized {
    fn get(&self, name: &str) -> &[f32] {
        self.f32s
            .get(name)
            .unwrap_or_else(|| panic!("no f32 tensor called {name}"))
    }

    fn get_i8(&self, name: &str) -> &[i8] {
        self.i8s
            .get(name)
            .unwrap_or_else(|| panic!("no i8 tensor called {name}"))
    }

    fn packed(&self, name: &str) -> bool {
        self.packed.contains(name)
    }
}

/// What `facekit quantize` does: every `<name>.weight` becomes `i8` data
/// plus `<name>.scales`, biases are copied, and each probed tensor gets
/// its `q.<node>` mapping from the range seen: a 16-bit `[scale, zero
/// point]` for a wide probe (all of the detector's), an 8-bit one for a
/// per-tensor probe, `[channel][scale, zero point]` for a per-channel one.
fn quantize_model(weights: &Tensors, ranges: &Ranges) -> Quantized {
    let mut out = Quantized {
        f32s: HashMap::new(),
        i8s: HashMap::new(),
        packed: HashSet::new(),
    };
    for name in weights.names() {
        let (shape, values) = weights.tensor(name);
        let Some(prefix) = name.strip_suffix(".weight") else {
            if name.ends_with(".bias") {
                out.f32s.insert(name.to_string(), values.to_vec());
            }
            continue;
        };
        let mut data = vec![0i8; values.len()];
        let mut scales;
        match weights.layout(name) {
            "HWC" => {
                let channels = shape[2];
                scales = vec![0.0f32; channels];
                quant::quantize_weight_channels(values, channels, &mut data, &mut scales);
            }
            "OHWI" | "OI" => {
                let channels = shape[0];
                scales = vec![0.0f32; channels];
                quant::quantize_weight_rows(
                    values,
                    values.len() / channels,
                    &mut data,
                    &mut scales,
                );
                if channels.is_multiple_of(pack::GROUP) {
                    let rows: Vec<u8> = data.iter().map(|&w| w as u8).collect();
                    let mut packed = vec![0u8; rows.len()];
                    pack::pack_rows(&rows, channels, rows.len() / channels, &mut packed);
                    data = packed.iter().map(|&w| w as i8).collect();
                    out.packed.insert(name.to_string());
                }
            }
            other => panic!("{name}: unexpected weight layout {other}"),
        }
        out.i8s.insert(name.to_string(), data);
        out.f32s.insert(format!("{prefix}.scales"), scales);
    }
    for (node, (granularity, ranges)) in ranges {
        let pairs = ranges
            .iter()
            .flat_map(|&(min, max)| {
                let mapping = if granularity.wide() {
                    Quant::from_range16(min, max)
                } else {
                    Quant::from_range(min, max)
                };
                [mapping.scale, mapping.zero_point as f32]
            })
            .collect();
        out.f32s.insert(format!("q.{node}"), pairs);
    }
    out
}

/// The golden input, converted to the kernels' layout.
fn golden_input(golden: &Tensors) -> Vec<f32> {
    let (shape, input) = golden.activation_nchw("input");
    assert_eq!(shape, INPUT_SHAPE);
    input
}

/// The golden input (bytes stored as `f32`) as the integer pass takes it.
fn input_i8(input: &[f32]) -> Vec<i8> {
    input
        .iter()
        .map(|value| (*value as i32 - 128) as i8)
        .collect()
}

/// The `f32` pass on the golden input: its trace, in graph order, and the
/// range of every probed tensor.
type Trace = Vec<(String, Shape, Vec<f32>)>;

/// Run the `f32` pass, keep its trace, and build the integer weights from
/// the ranges it probes.
fn calibrate(weights: &Tensors, input: &[f32]) -> (Trace, Quantized) {
    let mut scratch = vec![0.0f32; yunet::SCRATCH_LEN];
    let mut trace = Trace::new();
    let mut ranges = Ranges::new();
    yunet::forward_probed(
        weights,
        input,
        &mut scratch,
        |node, shape, values| trace.push((node.to_string(), shape, values.to_vec())),
        |node, granularity, values| {
            let channels = granularity.channels();
            let (_, entry) = ranges.entry(node.to_string()).or_insert_with(|| {
                (
                    granularity,
                    vec![(f32::INFINITY, f32::NEG_INFINITY); channels],
                )
            });
            for (index, &value) in values.iter().enumerate() {
                let range = &mut entry[index % channels];
                *range = (range.0.min(value), range.1.max(value));
            }
        },
    );
    (trace, quantize_model(weights, &ranges))
}

#[test]
fn every_stage_is_within_the_quantization_noise() {
    let weights = Tensors::load(WEIGHTS);
    let golden = Tensors::load(GOLDEN);
    let input = golden_input(&golden);
    let (reference, quantized) = calibrate(&weights, &input);

    let mut i16s = vec![0i16; int8::SCRATCH_I16_LEN + 8];
    let mut f32s = vec![0.0f32; int8::F32_SCRATCH_LEN];
    let mut padded = vec![0i8; int8::MODEL_WEIGHTS_LEN + 16];
    let mut plans = vec![GroupPlan::ZERO; int8::MODEL_PLANS];
    let model = int8::Model::compile(
        &quantized,
        int8::ModelStorage {
            weights: common::aligned(&mut padded),
            plans: &mut plans,
        },
    );
    let mut actual = Trace::new();
    let heads = model.forward_traced(
        &input_i8(&input),
        int8::Scratch::new(common::aligned(&mut i16s), &mut f32s),
        |node, shape, values| actual.push((node.to_string(), shape, values.to_vec())),
    );

    assert_eq!(actual.len(), reference.len(), "traced node count");
    let mut worst = f32::INFINITY;
    for ((node, shape, expected), (actual_node, actual_shape, values)) in
        reference.iter().zip(&actual)
    {
        assert_eq!(node, actual_node, "traced node order");
        assert_eq!(shape, actual_shape, "{node} shape");
        let snr = quant::snr_db(expected, values);
        println!(
            "{node:<12} {:>4}x{:<3}x{:<3} SNR {snr:>6.1} dB",
            shape.height, shape.width, shape.channels
        );
        worst = worst.min(snr);
    }
    println!("worst stage: {worst:.1} dB");
    assert!(
        worst >= MIN_SNR_DB,
        "a stage has an SNR of {worst} dB, below {MIN_SNR_DB}"
    );

    // The head outputs against the golden ones (the f32 pass matches them
    // to 1e-4, so they stand for it), for information: the `obj` outputs
    // are a handful of values near zero, where a ratio says little. The
    // decoded face is what counts.
    for head in &heads {
        for (name, values) in [
            ("cls", head.cls),
            ("obj", head.obj),
            ("bbox", head.bbox),
            ("kps", head.kps),
        ] {
            let name = format!("{name}_{}", head.stride);
            let snr = quant::snr_db(golden.flat(&name), values);
            println!("{name:<12} SNR {snr:>6.1} dB");
        }
    }
}

/// The largest coordinate difference between two faces: centre, size and
/// every landmark.
fn face_difference(a: &Face, b: &Face) -> f32 {
    let mut worst = 0.0f32;
    for (p, q) in [
        (a.centre(), b.centre()),
        ([a.width, a.height], [b.width, b.height]),
    ]
    .into_iter()
    .chain(a.landmarks.into_iter().zip(b.landmarks))
    {
        worst = worst.max((p[0] - q[0]).abs()).max((p[1] - q[1]).abs());
    }
    worst
}

#[test]
fn decodes_the_same_face_as_the_f32_pass() {
    let weights = Tensors::load(WEIGHTS);
    let golden = Tensors::load(GOLDEN);
    let input = golden_input(&golden);
    let (_, quantized) = calibrate(&weights, &input);

    let mut scratch = vec![0.0f32; yunet::SCRATCH_LEN];
    let heads = yunet::forward(&weights, &input, &mut scratch);
    let expected = decode(&heads, DEFAULT_SCORE_THRESHOLD, DEFAULT_NMS_THRESHOLD);

    let mut i16s = vec![0i16; int8::SCRATCH_I16_LEN + 8];
    let mut f32s = vec![0.0f32; int8::F32_SCRATCH_LEN];
    let mut padded = vec![0i8; int8::MODEL_WEIGHTS_LEN + 16];
    let mut plans = vec![GroupPlan::ZERO; int8::MODEL_PLANS];
    let model = int8::Model::compile(
        &quantized,
        int8::ModelStorage {
            weights: common::aligned(&mut padded),
            plans: &mut plans,
        },
    );
    let heads = model.forward(
        &input_i8(&input),
        int8::Scratch::new(common::aligned(&mut i16s), &mut f32s),
    );
    let actual = decode(&heads, DEFAULT_SCORE_THRESHOLD, DEFAULT_NMS_THRESHOLD);

    for (name, faces) in [("f32", &expected), ("int8", &actual)] {
        for face in faces.iter() {
            println!(
                "{name:<5} score {:.3}, centre ({:.2}, {:.2}), {:.2}x{:.2}, landmarks {:?}",
                face.score,
                face.centre()[0],
                face.centre()[1],
                face.width,
                face.height,
                face.landmarks
            );
        }
    }
    assert_eq!((expected.len(), actual.len()), (1, 1), "one face each");
    let (expected, actual) = (expected[0], actual[0]);
    let difference = face_difference(&expected, &actual);
    println!(
        "largest difference {difference:.2} px, score difference {:.3}",
        (expected.score - actual.score).abs()
    );
    assert!(
        (expected.score - actual.score).abs() <= MAX_SCORE_DIFFERENCE,
        "score {} vs {}",
        actual.score,
        expected.score
    );
    for (name, a, b) in [
        ("centre x", actual.centre()[0], expected.centre()[0]),
        ("centre y", actual.centre()[1], expected.centre()[1]),
        ("width", actual.width, expected.width),
        ("height", actual.height, expected.height),
    ] {
        assert!((a - b).abs() <= MAX_FACE_DIFFERENCE, "{name}: {a} vs {b}");
    }
    for (n, (a, b)) in actual.landmarks.iter().zip(&expected.landmarks).enumerate() {
        assert!(
            (a[0] - b[0]).abs() <= MAX_FACE_DIFFERENCE
                && (a[1] - b[1]).abs() <= MAX_FACE_DIFFERENCE,
            "landmark {n}: {a:?} vs {b:?}"
        );
    }
}

#[test]
fn the_i8_input_is_the_f32_input_minus_128() {
    // A 3x2 image with every kind of value, including 0 and 255.
    let pixels: [u8; 18] = [
        0, 255, 128, 1, 2, 3, 250, 127, 129, 10, 20, 30, 255, 0, 64, 200, 100, 50,
    ];
    let frame = RgbImage::new(&pixels, 3, 2);
    let mut expected = vec![0.0f32; INPUT_SHAPE.len()];
    detector_input(&frame, &mut expected);
    let mut actual = vec![0i8; INPUT_SHAPE.len()];
    detector_input_i8(&frame, &mut actual);
    for (index, (&a, &e)) in actual.iter().zip(&expected).enumerate() {
        assert_eq!(i32::from(a), e as i32 - 128, "value {index}");
        assert_eq!(
            int8::INPUT_QUANT.dequantize(a),
            e,
            "value {index} dequantized"
        );
    }
    // The frame's first pixel is (r 0, g 255, b 128): B, G, R order.
    assert_eq!(&actual[..3], &[0, 127, -128]);
    // The padding is black.
    assert_eq!(actual[INPUT_SHAPE.offset(3, 0)], -128);
    assert_eq!(actual[INPUT_SHAPE.len() - 1], -128);
}
