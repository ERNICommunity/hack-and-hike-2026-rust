//! MFN_S8_V1 against the interpreter of its `.espdl` graph
//! (`facekit golden-espdl`), on the fixture face, bit for bit.

use hack_and_hike_vision::{
    blob::Blob,
    image::RgbImage,
    nn::{
        mfn::{self, Model, Scratch},
        s8::Plan,
    },
};

/// The recognizer's weights, as the firmware carries them.
const WEIGHTS: &[u8] = include_bytes!("../../../assets/models/mfn_s8_v1.fkb");
/// The interpreter's input and embedding for `face_112x112.jpg`.
const GOLDEN: &[u8] = include_bytes!("fixtures/mfn.golden.fkb");
/// The aligned fixture face.
const FACE: &[u8] = include_bytes!("fixtures/face_112x112.jpg");

/// The fixture face's RGB bytes.
fn face() -> Vec<u8> {
    image::load_from_memory(FACE)
        .expect("the fixture decodes")
        .to_rgb8()
        .into_raw()
}

/// The embedding of `input` with buffers of the given sizes.
fn embed(input: &[i8], ring: usize, filtered: usize, staging: usize) -> Vec<i8> {
    let blob = Blob::parse(WEIGHTS).expect("the weights parse");
    let mut plans = vec![Plan::ZERO; mfn::MODEL_PLANS];
    let model = Model::compile(&blob, &mut plans);
    let (mut first, mut second) = (vec![0i8; mfn::TENSOR_LEN], vec![0i8; mfn::TENSOR_LEN]);
    let (mut ring, mut filtered, mut staging) =
        (vec![0i8; ring], vec![0i8; filtered], vec![0i8; staging]);
    let mut columns = vec![0i8; mfn::COLUMNS_LEN];
    let mut embedding = vec![0i8; mfn::EMBEDDING_LEN];
    let mut parts = Vec::new();
    model.forward_traced(
        input,
        Scratch {
            tensors: [&mut first, &mut second],
            ring: &mut ring,
            filtered: &mut filtered,
            staging: &mut staging,
            columns: &mut columns,
        },
        &mut embedding,
        |part| parts.push(part.to_string()),
    );
    assert_eq!(parts.len(), 17, "stem, fifteen blocks, head: {parts:?}");
    embedding
}

#[test]
fn input_matches_the_interpreter() {
    let golden = Blob::parse(GOLDEN).expect("the golden file parses");
    let rgb = face();
    let mut input = vec![0i8; 112 * 112 * 3];
    mfn::input_i8(&RgbImage::new(&rgb, 112, 112), &mut input);
    assert_eq!(input, golden.get("input").expect("input").i8_slice());
    assert_eq!(mfn::input_value(0), -64);
    assert_eq!(mfn::input_value(255), 64);
    assert_eq!(mfn::input_value(128), 0);
    assert_eq!(mfn::input_value(127), 0);
}

#[test]
fn embedding_matches_the_interpreter() {
    let golden = Blob::parse(GOLDEN).expect("the golden file parses");
    let input = golden.get("input").expect("input").i8_slice();
    let expected = golden.get("embedding").expect("embedding").i8_slice();
    let fast = embed(input, mfn::FAST_RING, mfn::FAST_FILTERED, mfn::FAST_STAGING);
    assert_eq!(fast, expected, "with bands of seven rows");
    let least = embed(input, mfn::MIN_RING, mfn::MIN_FILTERED, mfn::MIN_STAGING);
    assert_eq!(least, expected, "with the least buffers");
}

#[test]
fn the_network_has_its_published_size() {
    let blob = Blob::parse(WEIGHTS).expect("the weights parse");
    let mut plans = vec![Plan::ZERO; mfn::MODEL_PLANS];
    let model = Model::compile(&blob, &mut plans);
    let products = model.products();
    println!("{products} products per face");
    assert!((220_000_000..223_000_000).contains(&products), "{products}");
}
