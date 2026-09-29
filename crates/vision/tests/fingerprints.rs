//! The board's whole path on the fixture photo, pinned bit for bit.
//!
//! The other tests compare the integer networks with the `f32` ones
//! within a tolerance. That does not catch a change that moves a few
//! values by one step. This test does: it runs what the application runs
//! (the camera frame scaled down, the detector, the alignment, the
//! sharpness, the recognizer) with the firmware's own weights files, and
//! compares a fingerprint of every result with the one recorded when the
//! numbers were last changed on purpose.
//!
//! A change that is meant to keep the numbers (a faster loop, another
//! order of the work) must leave every fingerprint alone. A change that is
//! meant to move them (another kernel, another rounding) fails here; then
//! regenerate the impostor bank and the thresholds with `facekit`, and
//! write the new fingerprints down. The test prints them all.

use hack_and_hike_vision::{
    align::{CROP_SIZE, align_face, recognizer_input_i8, source_region},
    blob::Blob,
    detect::{
        CONTENT_HEIGHT, CONTENT_WIDTH, DEFAULT_NMS_THRESHOLD, DEFAULT_SCORE_THRESHOLD, DOWNSCALE,
        decode, detector_input_i8,
    },
    image::{
        GrayImageMut, Rgb565Frame, RgbImageMut, downscale_to_rgb, downscale_to_rgb_within,
        rgb_to_gray,
    },
    nn::{
        BlobWeights, check, edgeface,
        lanes::{GeluTable, GroupPlan, NormPlan},
        pack, yunet,
    },
    quality::laplacian_variance,
};

/// The recognizer's integer weights, as the firmware carries them.
const EDGEFACE: &[u8] = include_bytes!("../../../assets/models/edgeface_xxs.int8.fkb");
/// The detector's integer weights, as the firmware carries them.
const YUNET: &[u8] = include_bytes!("../../../assets/models/yunet.int8.fkb");
/// The fixture face at the board's frame size.
const PHOTO: &[u8] = include_bytes!("fixtures/face_320x240.jpg");

/// The fingerprints since faceid-12, where the detector moved to the lane
/// kernels (faceid-9 to faceid-11 had the same numbers up to the
/// detector: heads `0x470c7ea90ef6060d`).
const EXPECTED: [(&str, u64); 9] = [
    ("frame scaled down by 4", 0xe72f_b550_889e_b1ad),
    ("frame scaled down by 2", 0x9309_0712_1d5b_5551),
    ("detector heads", 0xb43d_9b2b_2382_ce4e),
    ("best face", 0xa232_57ca_30bb_d6fc),
    ("aligned crop", 0x8888_a8dd_7225_dc2d),
    ("gray crop", 0xf560_5afd_1cb4_1082),
    ("sharpness", 0x8534_7d80_58ad_5088),
    ("recognizer input", 0xc0b8_36bc_d425_edce),
    ("embedding", 0x6763_5a7b_e5ec_460a),
];

/// FNV-1a over bytes, as [`check::fingerprint`] over the bits of `f32`
/// values: small, and any changed bit changes it.
fn fingerprint(bytes: impl IntoIterator<Item = u8>) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// The fingerprint of `f32` values, by their bits: the board's.
fn fingerprint_f32(values: &[f32]) -> u64 {
    check::fingerprint(values.iter().copied())
}

/// Memory on a 16-byte boundary, as the board's PSRAM buffers are.
struct Aligned {
    /// The storage; `u128` has the alignment.
    words: Vec<u128>,
    /// The bytes in use.
    len: usize,
}

impl Aligned {
    /// `len` zero bytes.
    fn new(len: usize) -> Self {
        Self {
            words: vec![0; len.div_ceil(16)],
            len,
        }
    }

    /// The bytes.
    fn bytes(&self) -> &[u8] {
        &bytemuck::cast_slice(&self.words)[..self.len]
    }

    /// The bytes, to write.
    fn bytes_mut(&mut self) -> &mut [u8] {
        &mut bytemuck::cast_slice_mut(&mut self.words)[..self.len]
    }

    /// The memory as `i16` values.
    fn i16s_mut(&mut self) -> &mut [i16] {
        let len = self.len / 2;
        &mut bytemuck::cast_slice_mut(&mut self.words)[..len]
    }
}

/// A copy of a weights file with its weights grouped for the vector
/// kernels, as the board makes it at start.
fn packed_copy(file: &[u8]) -> Aligned {
    // The file itself must be aligned for `Blob` to lend `f32` slices.
    let mut source = Aligned::new(file.len());
    source.bytes_mut().copy_from_slice(file);
    let blob = Blob::parse(source.bytes()).expect("a valid weights file");
    let mut copy = Aligned::new(pack::packed_len(&blob));
    pack::pack_all(&blob, copy.bytes_mut());
    copy
}

/// The fixture photo as a big-endian RGB565 frame, as the camera delivers.
fn photo_as_rgb565() -> Vec<u8> {
    let photo = image::load_from_memory(PHOTO)
        .expect("fixture JPEG")
        .to_rgb8();
    assert_eq!((photo.width(), photo.height()), (320, 240));
    let mut frame = Vec::with_capacity(320 * 240 * 2);
    for rgb in photo.as_raw().chunks_exact(3) {
        let value =
            (u16::from(rgb[0] >> 3) << 11) | (u16::from(rgb[1] >> 2) << 5) | u16::from(rgb[2] >> 3);
        frame.extend_from_slice(&value.to_be_bytes());
    }
    frame
}

#[test]
fn the_board_path_is_bit_for_bit_what_it_was() {
    let mut actual: Vec<(&str, u64)> = Vec::new();
    let frame_bytes = photo_as_rgb565();
    let frame = Rgb565Frame::new(&frame_bytes, 320, 240);

    // The detector's input and its run.
    let mut small = vec![0u8; CONTENT_WIDTH * CONTENT_HEIGHT * 3];
    let mut small_image = RgbImageMut::new(&mut small, CONTENT_WIDTH, CONTENT_HEIGHT);
    downscale_to_rgb(&frame, DOWNSCALE, &mut small_image);
    let mut detector_input = vec![0i8; yunet::INPUT_SHAPE.len()];
    detector_input_i8(&small_image.as_image(), &mut detector_input);
    actual.push(("frame scaled down by 4", fingerprint(small.iter().copied())));

    let detector_file = packed_copy(YUNET);
    let detector = BlobWeights::new(detector_file.bytes()).expect("the detector's weights");
    let mut detector_weights = Aligned::new(yunet::int8::MODEL_WEIGHTS_LEN);
    let mut detector_plans = vec![GroupPlan::ZERO; yunet::int8::MODEL_PLANS];
    let detector_model = yunet::int8::Model::compile(
        &detector,
        yunet::int8::ModelStorage {
            weights: bytemuck::cast_slice_mut(detector_weights.bytes_mut()),
            plans: &mut detector_plans,
        },
    );
    let mut detector_i16 = Aligned::new(yunet::int8::SCRATCH_I16_LEN * 2);
    let mut detector_f32 = vec![0.0f32; yunet::int8::F32_SCRATCH_LEN];
    // Twice, as the board runs one model on frame after frame.
    for _ in 0..2 {
        detector_model.forward(
            &detector_input,
            yunet::int8::Scratch::new(detector_i16.i16s_mut(), &mut detector_f32),
        );
    }
    let heads = detector_model.forward(
        &detector_input,
        yunet::int8::Scratch::new(detector_i16.i16s_mut(), &mut detector_f32),
    );
    let head_values: Vec<f32> = heads
        .iter()
        .flat_map(|head| [head.cls, head.obj, head.bbox, head.kps])
        .flatten()
        .copied()
        .collect();
    actual.push(("detector heads", fingerprint_f32(&head_values)));
    let faces = decode(&heads, DEFAULT_SCORE_THRESHOLD, DEFAULT_NMS_THRESHOLD);
    let face = faces
        .best()
        .expect("the fixture face")
        .scaled(DOWNSCALE as f32);
    let mut face_values = vec![face.score, face.x, face.y, face.width, face.height];
    face_values.extend(face.landmarks.iter().flatten());
    actual.push(("best face", fingerprint_f32(&face_values)));

    // The recognizer's input.
    let mut half = vec![0u8; 160 * 120 * 3];
    let mut half_image = RgbImageMut::new(&mut half, 160, 120);
    downscale_to_rgb(&frame, 2, &mut half_image);
    let landmarks = face.landmarks.map(|[x, y]| [x / 2.0, y / 2.0]);
    let mut crop = vec![0u8; CROP_SIZE * CROP_SIZE * 3];
    let mut crop_image = RgbImageMut::new(&mut crop, CROP_SIZE, CROP_SIZE);
    align_face(&landmarks, &half_image.as_image(), &mut crop_image).expect("a transform");
    // The application scales down only the part that the alignment
    // reads, into a buffer that holds the frame before: the crop must not
    // change.
    let (columns, rows) = source_region(&landmarks, 160, 120).expect("a region");
    println!(
        "the alignment reads columns {columns:?} and rows {rows:?} of 160x120: {} % of the pixels",
        100 * columns.len() * rows.len() / (160 * 120)
    );
    let mut partial = vec![0x5Au8; 160 * 120 * 3];
    let mut partial_image = RgbImageMut::new(&mut partial, 160, 120);
    downscale_to_rgb_within(&frame, 2, &mut partial_image, columns, rows);
    let mut partial_crop = vec![0u8; CROP_SIZE * CROP_SIZE * 3];
    align_face(
        &landmarks,
        &partial_image.as_image(),
        &mut RgbImageMut::new(&mut partial_crop, CROP_SIZE, CROP_SIZE),
    )
    .expect("a transform");
    assert_eq!(
        partial_crop,
        crop_image.data(),
        "the crop from the partly scaled frame"
    );

    let mut gray = vec![0u8; CROP_SIZE * CROP_SIZE];
    let mut gray_image = GrayImageMut::new(&mut gray, CROP_SIZE, CROP_SIZE);
    rgb_to_gray(&crop_image.as_image(), &mut gray_image);
    let sharpness = laplacian_variance(&gray_image.as_image());
    let mut recognizer_input = vec![0i8; CROP_SIZE * CROP_SIZE * 3];
    recognizer_input_i8(&crop_image.as_image(), &mut recognizer_input);
    actual.push(("frame scaled down by 2", fingerprint(half.iter().copied())));
    actual.push(("aligned crop", fingerprint(crop.iter().copied())));
    actual.push(("gray crop", fingerprint(gray.iter().copied())));
    actual.push(("sharpness", fingerprint_f32(&[sharpness])));
    actual.push((
        "recognizer input",
        fingerprint(recognizer_input.iter().map(|&value| value as u8)),
    ));

    // The recognizer.
    let recognizer_file = packed_copy(EDGEFACE);
    let recognizer = BlobWeights::new(recognizer_file.bytes()).expect("the recognizer's weights");
    let mut gelu_storage = vec![0i16; GeluTable::LEN];
    let gelu = GeluTable::build(&mut gelu_storage);
    let mut plans = vec![GroupPlan::ZERO; edgeface::int8::MODEL_PLANS];
    let mut norm_plans =
        vec![NormPlan::new(&[1.0; 8], &[0.0; 8], &[1.0; 8]); edgeface::int8::MODEL_NORM_PLANS];
    let mut model_weights = Aligned::new(edgeface::int8::MODEL_WEIGHTS_LEN);
    let mut model_constants = Aligned::new(edgeface::int8::MODEL_CONSTANTS_LEN * 2);
    let mut model_wide = Aligned::new(edgeface::int8::MODEL_WIDE_LEN * 2);
    let model = edgeface::int8::Model::compile(
        &recognizer,
        edgeface::int8::ModelStorage {
            plans: &mut plans,
            norm_plans: &mut norm_plans,
            weights: bytemuck::cast_slice_mut(model_weights.bytes_mut()),
            constants: model_constants.i16s_mut(),
            wide: model_wide.i16s_mut(),
        },
    );
    let mut recognizer_i16 = Aligned::new(edgeface::int8::SCRATCH_I16_LEN * 2);
    let mut recognizer_f32 = vec![0.0f32; edgeface::int8::SCRATCH_F32_LEN];
    let mut hidden_strip = Aligned::new(edgeface::int8::HIDDEN_STRIP_LEN * 2);
    let mut embedding = vec![0.0f32; edgeface::EMBEDDING_LEN];
    // Twice, as the board runs one model on face after face: the second
    // pass must not depend on what the first left in the buffers. With
    // the board's strip of the MLP's hidden tensor.
    for _ in 0..2 {
        model.forward(
            &gelu,
            &recognizer_input,
            edgeface::int8::Scratch::new(recognizer_i16.i16s_mut(), &mut recognizer_f32)
                .with_hidden(hidden_strip.i16s_mut()),
            &mut embedding,
        );
    }
    actual.push(("embedding", fingerprint_f32(&embedding)));

    println!(
        "face: score {:.3}, box ({:.1}, {:.1}) {:.1}x{:.1}; sharpness {sharpness:.1}",
        face.score, face.x, face.y, face.width, face.height
    );
    for (name, value) in &actual {
        println!("    (\"{name}\", {value:#018x}),");
    }
    for (name, expected) in EXPECTED {
        let (_, value) = actual
            .iter()
            .find(|(actual_name, _)| *actual_name == name)
            .unwrap_or_else(|| panic!("no fingerprint called {name}"));
        assert_eq!(
            *value, expected,
            "{name}: {value:#018x}, was {expected:#018x}"
        );
    }
}

#[test]
fn the_networks_on_made_up_inputs_give_the_pinned_fingerprints() {
    // What the application checks on the board when it starts.
    let detector_file = packed_copy(YUNET);
    let detector = BlobWeights::new(detector_file.bytes()).expect("the detector's weights");
    let mut detector_weights = Aligned::new(yunet::int8::MODEL_WEIGHTS_LEN);
    let mut detector_plans = vec![GroupPlan::ZERO; yunet::int8::MODEL_PLANS];
    let detector_model = yunet::int8::Model::compile(
        &detector,
        yunet::int8::ModelStorage {
            weights: bytemuck::cast_slice_mut(detector_weights.bytes_mut()),
            plans: &mut detector_plans,
        },
    );
    let mut input = vec![0i8; yunet::INPUT_SHAPE.len()];
    check::noise(check::DETECTOR_SEED, &mut input);
    let mut detector_i16 = Aligned::new(yunet::int8::SCRATCH_I16_LEN * 2);
    let mut detector_f32 = vec![0.0f32; yunet::int8::F32_SCRATCH_LEN];
    let heads = detector_model.forward(
        &input,
        yunet::int8::Scratch::new(detector_i16.i16s_mut(), &mut detector_f32),
    );
    let detector_print = check::fingerprint(
        heads
            .iter()
            .flat_map(|head| [head.cls, head.obj, head.bbox, head.kps])
            .flatten()
            .copied(),
    );

    let recognizer_file = packed_copy(EDGEFACE);
    let recognizer = BlobWeights::new(recognizer_file.bytes()).expect("the recognizer's weights");
    let mut gelu_storage = vec![0i16; GeluTable::LEN];
    let gelu = GeluTable::build(&mut gelu_storage);
    let mut plans = vec![GroupPlan::ZERO; edgeface::int8::MODEL_PLANS];
    let mut norm_plans =
        vec![NormPlan::new(&[1.0; 8], &[0.0; 8], &[1.0; 8]); edgeface::int8::MODEL_NORM_PLANS];
    let mut model_weights = Aligned::new(edgeface::int8::MODEL_WEIGHTS_LEN);
    let mut model_constants = Aligned::new(edgeface::int8::MODEL_CONSTANTS_LEN * 2);
    let mut model_wide = Aligned::new(edgeface::int8::MODEL_WIDE_LEN * 2);
    let model = edgeface::int8::Model::compile(
        &recognizer,
        edgeface::int8::ModelStorage {
            plans: &mut plans,
            norm_plans: &mut norm_plans,
            weights: bytemuck::cast_slice_mut(model_weights.bytes_mut()),
            constants: model_constants.i16s_mut(),
            wide: model_wide.i16s_mut(),
        },
    );
    let mut input = vec![0i8; CROP_SIZE * CROP_SIZE * 3];
    check::noise(check::RECOGNIZER_SEED, &mut input);
    let mut recognizer_i16 = Aligned::new(edgeface::int8::SCRATCH_I16_LEN * 2);
    let mut recognizer_f32 = vec![0.0f32; edgeface::int8::SCRATCH_F32_LEN];
    let mut embedding = vec![0.0f32; edgeface::EMBEDDING_LEN];
    model.forward(
        &gelu,
        &input,
        edgeface::int8::Scratch::new(recognizer_i16.i16s_mut(), &mut recognizer_f32),
        &mut embedding,
    );
    let recognizer_print = check::fingerprint(embedding.iter().copied());
    // The MLP's hidden tensor in strips, as short as allowed, of the
    // board's length and of a length that leaves a short last strip: the
    // same numbers.
    for strip in [
        edgeface::int8::MIN_HIDDEN_LEN,
        edgeface::int8::HIDDEN_STRIP_LEN,
        5000,
    ] {
        let mut hidden = Aligned::new(strip * 2);
        model.forward(
            &gelu,
            &input,
            edgeface::int8::Scratch::new(recognizer_i16.i16s_mut(), &mut recognizer_f32)
                .with_hidden(hidden.i16s_mut()),
            &mut embedding,
        );
        assert_eq!(
            check::fingerprint(embedding.iter().copied()),
            recognizer_print,
            "the recognizer with a hidden strip of {strip}"
        );
    }

    println!("pub const DETECTOR: u64 = {detector_print:#018x};");
    println!("pub const RECOGNIZER: u64 = {recognizer_print:#018x};");
    assert_eq!(
        detector_print,
        check::DETECTOR,
        "the detector's fingerprint"
    );
    assert_eq!(
        recognizer_print,
        check::RECOGNIZER,
        "the recognizer's fingerprint"
    );
}
