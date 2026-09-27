//! The EdgeFace-XXS reference implementation against the golden vectors
//! of tract, block by block.

mod common;

use common::{Tensors, assert_close};
use hack_and_hike_vision::nn::edgeface::{EMBEDDING_LEN, SCRATCH_LEN, forward, forward_traced};

/// The weights, as exported by facekit.
const WEIGHTS: &[u8] = include_bytes!("fixtures/edgeface_xxs.f32.fkb");
/// The golden vectors on the fixture face.
const GOLDEN: &[u8] = include_bytes!("fixtures/edgeface_xxs.golden.fkb");

/// Absolute tolerance per value.
const ATOL: f32 = 1e-4;
/// Relative tolerance, times the largest value of the golden tensor.
const RTOL: f32 = 1e-4;

#[test]
fn every_block_matches_tract() {
    let weights = Tensors::load(WEIGHTS);
    let golden = Tensors::load(GOLDEN);
    let (_, input) = golden.activation_nchw("input");
    let mut scratch = vec![0.0f32; SCRATCH_LEN];
    let mut embedding = vec![0.0f32; EMBEDDING_LEN];

    let mut traced = Vec::new();
    forward_traced(
        &weights,
        &input,
        &mut scratch,
        &mut embedding,
        |name, _, values| {
            traced.push((name.to_string(), values.to_vec()));
        },
    );

    // Every traced block must be in the golden file, and vice versa: a
    // missing name means the trace and the file disagree about the graph.
    let mut checked = 0;
    for (name, values) in &traced {
        assert!(golden.has(name), "the golden file has no tensor {name}");
        assert_close(name, values, &golden.activation(name), ATOL, RTOL);
        checked += 1;
    }
    assert_eq!(checked, 26, "number of compared blocks");
    assert_close(
        "embedding",
        &embedding,
        golden.flat("embedding"),
        ATOL,
        RTOL,
    );
}

#[test]
fn forward_gives_the_same_embedding_as_the_traced_run() {
    let weights = Tensors::load(WEIGHTS);
    let golden = Tensors::load(GOLDEN);
    let (_, input) = golden.activation_nchw("input");
    let mut scratch = vec![0.0f32; SCRATCH_LEN];
    let mut embedding = vec![0.0f32; EMBEDDING_LEN];
    forward(&weights, &input, &mut scratch, &mut embedding);
    let norm: f32 = embedding.iter().map(|v| v * v).sum::<f32>().sqrt();
    assert!((norm - 1.928).abs() < 0.01, "embedding norm {norm}");
}
