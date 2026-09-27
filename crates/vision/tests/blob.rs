//! The FKB1 reader, on a hand-made file and on the fixtures that facekit
//! wrote.
//!
//! The fixtures in `tests/fixtures/` are the weights of both models and
//! the golden vectors of both models. `tests/fixtures/README.md` says how
//! to make them again.

use hack_and_hike_vision::blob::{
    Blob, BlobError, DATA_ALIGN, DataType, ENTRY_LEN, HEADER_LEN, MAGIC,
};

/// The weights of EdgeFace-XXS, as exported by facekit.
const EDGEFACE_WEIGHTS: &[u8] = include_bytes!("fixtures/edgeface_xxs.f32.fkb");
/// Reference outputs of EdgeFace-XXS on `face_112x112.jpg`.
const EDGEFACE_GOLDEN: &[u8] = include_bytes!("fixtures/edgeface_xxs.golden.fkb");
/// The weights of YuNet, as exported by facekit.
const YUNET_WEIGHTS: &[u8] = include_bytes!("fixtures/yunet.f32.fkb");
/// Reference outputs of YuNet on `face_320x240.jpg`.
const YUNET_GOLDEN: &[u8] = include_bytes!("fixtures/yunet.golden.fkb");

/// A file with one entry: `name`, `values` as f32, dimensions `dims`.
fn one_entry_file(
    name: &str,
    dims: &[u32],
    values: &[f32],
    offset_override: Option<u32>,
) -> Vec<u8> {
    let data_offset = (HEADER_LEN + ENTRY_LEN).next_multiple_of(DATA_ALIGN) as u32;
    let mut file = Vec::new();
    file.extend_from_slice(&MAGIC);
    file.extend_from_slice(&1u32.to_le_bytes());
    file.extend_from_slice(&[0u8; 8]);
    let mut entry = [0u8; ENTRY_LEN];
    entry[..name.len()].copy_from_slice(name.as_bytes());
    entry[64] = 0; // f32
    entry[65] = dims.len() as u8;
    for (axis, dim) in dims.iter().enumerate() {
        entry[68 + axis * 4..72 + axis * 4].copy_from_slice(&dim.to_le_bytes());
    }
    entry[84..88].copy_from_slice(&offset_override.unwrap_or(data_offset).to_le_bytes());
    entry[88..92].copy_from_slice(&((values.len() * 4) as u32).to_le_bytes());
    entry[100..103].copy_from_slice(b"OI\0");
    file.extend_from_slice(&entry);
    file.resize(data_offset as usize, 0);
    for value in values {
        file.extend_from_slice(&value.to_le_bytes());
    }
    file
}

#[test]
fn reads_a_hand_made_file() {
    let file = one_entry_file(
        "tiny.weight",
        &[2, 3],
        &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
        None,
    );
    let blob = Blob::parse(&file).expect("valid file");
    assert_eq!(blob.len(), 1);
    let entry = blob.get("tiny.weight").expect("entry by name");
    assert_eq!(entry.name, "tiny.weight");
    assert_eq!(entry.layout, "OI");
    assert_eq!(entry.data_type, DataType::F32);
    assert_eq!(entry.shape(), &[2, 3]);
    assert_eq!(entry.element_count(), 6);
    assert_eq!(
        entry.f32s().collect::<Vec<_>>(),
        [1.0, 2.0, 3.0, 4.0, 5.0, 6.0]
    );
    let mut copy = [0.0f32; 6];
    entry.read_f32s(&mut copy);
    assert_eq!(copy[5], 6.0);
    assert!(blob.get("missing").is_none());
}

#[test]
fn rejects_broken_files() {
    assert_eq!(Blob::parse(b"FKB").err(), Some(BlobError::TooShort));
    let mut file = one_entry_file("x", &[1], &[1.0], None);
    file[0] = b'X';
    assert_eq!(Blob::parse(&file).err(), Some(BlobError::BadMagic));

    // The table announces one entry, but the file ends before it.
    let file = one_entry_file("x", &[1], &[1.0], None);
    assert_eq!(
        Blob::parse(&file[..HEADER_LEN + 10]).err(),
        Some(BlobError::TooShort)
    );

    // A data offset that is not a multiple of 16.
    let file = one_entry_file(
        "x",
        &[1],
        &[1.0],
        Some(HEADER_LEN as u32 + ENTRY_LEN as u32 + 4),
    );
    assert_eq!(Blob::parse(&file).err(), Some(BlobError::BadRange(0)));

    // Data past the end of the file.
    let file = one_entry_file("x", &[1], &[1.0], Some(1 << 20));
    assert_eq!(Blob::parse(&file).err(), Some(BlobError::BadRange(0)));

    // Dimensions that do not match the byte length.
    let file = one_entry_file("x", &[2], &[1.0], None);
    assert_eq!(Blob::parse(&file).err(), Some(BlobError::BadRange(0)));

    // An empty name.
    let file = one_entry_file("", &[1], &[1.0], None);
    assert_eq!(Blob::parse(&file).err(), Some(BlobError::BadName(0)));
}

#[test]
fn edgeface_weights_have_the_expected_shapes() {
    let blob = Blob::parse(EDGEFACE_WEIGHTS).expect("valid weights file");
    assert_eq!(blob.len(), 161);
    let stem = blob.get("stem.0.weight").expect("stem weight");
    assert_eq!(
        (stem.layout, stem.shape()),
        ("OHWI", &[24usize, 4, 4, 3][..])
    );
    let depthwise = blob
        .get("stages.3.blocks.0.conv_dw.weight")
        .expect("depthwise weight");
    assert_eq!(
        (depthwise.layout, depthwise.shape()),
        ("HWC", &[9usize, 9, 168][..])
    );
    let linear = blob
        .get("stages.0.blocks.0.mlp.fc1.weight")
        .expect("linear weight");
    assert_eq!((linear.layout, linear.shape()), ("OI", &[96usize, 24][..]));
    let head = blob.get("head.fc.weight").expect("head weight");
    assert_eq!((head.layout, head.shape()), ("OI", &[512usize, 168][..]));
    let temperature = blob
        .get("stages.1.blocks.1.xca.temperature")
        .expect("temperature");
    assert_eq!(temperature.shape(), &[4]);
    let positional = blob
        .get("stages.1.blocks.1.pos_embd.constant")
        .expect("folded constant");
    assert_eq!(
        (positional.layout, positional.shape()),
        ("HWC", &[14usize, 14, 48][..])
    );
    assert!(positional.f32s().all(f32::is_finite));

    // Every tensor is f32 and finite: a broken export would show up here.
    let total: usize = blob.entries().map(|entry| entry.element_count()).sum();
    assert_eq!(
        total - positional.element_count(),
        1_244_744,
        "parameter count of EdgeFace-XXS"
    );
    for entry in blob.entries() {
        assert_eq!(entry.data_type, DataType::F32, "{}", entry.name);
        assert!(
            entry.f32s().all(f32::is_finite),
            "{} has a non-finite value",
            entry.name
        );
    }
}

#[test]
fn yunet_weights_have_the_expected_shapes() {
    let blob = Blob::parse(YUNET_WEIGHTS).expect("valid weights file");
    let stem = blob.get("Conv_0.weight").expect("stem weight");
    assert_eq!(
        (stem.layout, stem.shape()),
        ("OHWI", &[16usize, 3, 3, 3][..])
    );
    let depthwise = blob.get("Conv_3.weight").expect("depthwise weight");
    assert_eq!(
        (depthwise.layout, depthwise.shape()),
        ("HWC", &[3usize, 3, 16][..])
    );
    let head = blob
        .get("bbox_head.multi_level_kps.0.conv1.weight")
        .expect("head weight");
    assert_eq!(
        (head.layout, head.shape()),
        ("OHWI", &[10usize, 1, 1, 64][..])
    );
    // The ONNX file has 53,121 initializer values. 17 of them are not
    // weights and are not exported: the two `Resize` scale vectors (8) and
    // the `Reshape` shape vectors (9), which the firmware hard-codes.
    let total: usize = blob.entries().map(|entry| entry.element_count()).sum();
    assert_eq!(total, 53_104, "parameter count of YuNet");
}

#[test]
fn golden_files_describe_the_expected_runs() {
    let edgeface = Blob::parse(EDGEFACE_GOLDEN).expect("valid golden file");
    let image = edgeface.get("image_rgb").expect("image");
    assert_eq!(
        (image.data_type, image.shape()),
        (DataType::U8, &[112usize, 112, 3][..])
    );
    let input = edgeface.get("input").expect("input");
    assert_eq!(input.shape(), &[1, 3, 112, 112]);
    assert!(input.f32s().all(|value| (-1.0..=1.0).contains(&value)));
    let embedding = edgeface.get("embedding").expect("embedding");
    assert_eq!(embedding.shape(), &[1, 512]);
    let norm: f32 = embedding
        .f32s()
        .map(|value| value * value)
        .sum::<f32>()
        .sqrt();
    assert!(norm.is_finite() && norm > 0.1, "embedding norm {norm}");
    assert!(
        edgeface.get("stages.3.blocks.1.Add_4").is_some(),
        "last block boundary"
    );

    let yunet = Blob::parse(YUNET_GOLDEN).expect("valid golden file");
    let image = yunet.get("image_rgb").expect("image");
    assert_eq!(image.shape(), &[64, 96, 3]);
    let input = yunet.get("input").expect("input");
    assert_eq!(input.shape(), &[1, 3, 64, 96]);
    assert!(input.f32s().all(|value| (0.0..=255.0).contains(&value)));
    for (name, anchors, values) in [("cls_8", 96, 1), ("bbox_16", 24, 4), ("kps_32", 6, 10)] {
        let output = yunet.get(name).expect(name);
        assert_eq!(output.shape(), &[1, anchors, values], "{name}");
    }

    // The photo shows a face that fills the frame height, so the stride-8
    // head must be confident somewhere near the middle of the 12x8 anchor
    // grid. The score is the square root of `cls * obj`, as in OpenCV.
    let cls: Vec<f32> = yunet.get("cls_8").expect("cls_8").f32s().collect();
    let obj: Vec<f32> = yunet.get("obj_8").expect("obj_8").f32s().collect();
    let (best, score) = cls
        .iter()
        .zip(&obj)
        .map(|(c, o)| (c * o).max(0.0).sqrt())
        .enumerate()
        .fold((0, 0.0f32), |best, (index, score)| {
            if score > best.1 { (index, score) } else { best }
        });
    assert!(score > 0.8, "best face score {score} at anchor {best}");
    let (row, column) = (best / 12, best % 12);
    assert!(
        (2..=5).contains(&row) && (3..=6).contains(&column),
        "face at row {row}, column {column}"
    );
}
