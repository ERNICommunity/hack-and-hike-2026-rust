//! The integer recognizer against the `f32` one: the mappings, then the
//! whole network on the fixture face with the integer weights that
//! `facekit quantize` produced.

mod common;

use common::Tensors;
use hack_and_hike_vision::{
    align::{recognizer_input, recognizer_input_i8},
    image::RgbImage,
    nn::{
        edgeface,
        quant::{Quant, snr_db},
    },
};

/// The `f32` weights.
const WEIGHTS_F32: &[u8] = include_bytes!("fixtures/edgeface_xxs.f32.fkb");
/// The integer weights, from `facekit quantize` on the calibration crops.
const WEIGHTS_I8: &[u8] = include_bytes!("../../../assets/models/edgeface_xxs.int8.fkb");
/// The golden run: its `image_rgb` is the fixture face crop.
const GOLDEN: &[u8] = include_bytes!("fixtures/edgeface_xxs.golden.fkb");

/// Cosine similarity.
fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norm = |v: &[f32]| v.iter().map(|x| x * x).sum::<f32>().sqrt();
    dot / (norm(a) * norm(b))
}

#[test]
fn mappings_round_trip_and_saturate() {
    let quant = Quant::from_range(-2.0, 6.0);
    // 0.0 is exactly representable: padding and ReLU stay exact.
    assert_eq!(quant.dequantize(quant.quantize(0.0)), 0.0);
    for value in [-2.0f32, -1.0, 0.5, 3.0, 6.0] {
        let error = (quant.dequantize(quant.quantize(value)) - value).abs();
        assert!(error <= quant.scale / 2.0 + 1e-6, "{value}: error {error}");
    }
    assert_eq!(quant.quantize(100.0), 127);
    assert_eq!(quant.quantize(-100.0), -128);
    // A range on one side of zero is widened to include zero.
    let positive = Quant::from_range(1.0, 5.0);
    assert_eq!(positive.dequantize(positive.quantize(0.0)), 0.0);
    assert_eq!(positive.zero_point, -128);
}

#[test]
fn the_integer_network_agrees_with_the_float_one() {
    let f32s = Tensors::load(WEIGHTS_F32);
    let i8s = Tensors::load(WEIGHTS_I8);
    let golden = Tensors::load(GOLDEN);
    let (dims, image) = golden.u8_tensor("image_rgb");
    let crop = RgbImage::new(image, dims[1], dims[0]);
    let mut input = vec![0.0f32; 112 * 112 * 3];
    let mut input_i8 = vec![0i8; 112 * 112 * 3];
    recognizer_input(&crop, &mut input);
    recognizer_input_i8(&crop, &mut input_i8);

    let mut scratch = vec![0.0f32; edgeface::SCRATCH_LEN];
    let mut reference = vec![0.0f32; edgeface::EMBEDDING_LEN];
    let mut traced = Vec::new();
    edgeface::forward_traced(
        &f32s,
        &input,
        &mut scratch,
        &mut reference,
        |name, _, values| {
            traced.push((name.to_string(), values.to_vec()));
        },
    );

    let mut embedding = vec![0.0f32; edgeface::EMBEDDING_LEN];
    // The integer pass traces the same blocks under the same names, except
    // the LayerNorm output of the head, which it folds into the last layer.
    let mut compared = 0;
    let mut worst = f32::INFINITY;
    let mut i8s = i8s;
    i8s.pack_for_lanes();
    common::Runner::new().forward_traced(&i8s, &input_i8, &mut embedding, |name, _, values| {
        let (_, expected) = traced
            .iter()
            .find(|(expected, _)| expected == name)
            .unwrap_or_else(|| panic!("the f32 pass does not trace {name}"));
        let snr = snr_db(expected, values);
        println!("{name:<44} {snr:6.1} dB");
        worst = worst.min(snr);
        compared += 1;
    });
    assert_eq!(
        compared,
        traced.len() - 1,
        "every block but the folded LayerNorm output traced"
    );
    // What the application sees is the embedding, checked below by
    // cosine; the blocks are printed for the record.
    assert!(worst >= 15.0, "worst block SNR {worst} dB");

    let similarity = cosine(&reference, &embedding);
    let versus_golden = cosine(golden.flat("embedding"), &embedding);
    println!("embedding cosine: int8 vs f32 {similarity:.4}, int8 vs tract {versus_golden:.4}");
    // `facekit quantize` measured 0.996 worst, 0.998 mean over 64 faces.
    assert!(similarity >= 0.995, "cosine {similarity}");
    assert!(versus_golden >= 0.995, "cosine {versus_golden}");
}
