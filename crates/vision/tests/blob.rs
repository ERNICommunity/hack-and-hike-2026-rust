//! The FKB1 reader, on a hand-made file and on the fixtures that facekit
//! wrote.
//!
//! The fixtures are the weights of both models and their golden vectors
//! (the recognizer's weights are the firmware's own, in `assets/models`). `tests/fixtures/README.md` says how
//! to make them again.

use hack_and_hike_vision::blob::{
    Blob, BlobError, DATA_ALIGN, DataType, ENTRY_LEN, HEADER_LEN, MAGIC,
};

/// The weights of MFN_S8_V1, as `facekit import-espdl` wrote them.
const MFN_WEIGHTS: &[u8] = include_bytes!("../../../assets/models/mfn_s8_v1.fkb");
/// Reference input and embedding of MFN_S8_V1 on `face_112x112.jpg`.
const MFN_GOLDEN: &[u8] = include_bytes!("fixtures/mfn.golden.fkb");
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
fn mfn_weights_have_the_expected_shapes() {
    let blob = Blob::parse(MFN_WEIGHTS).expect("valid weights file");
    // Fifty layers with a weight, a bias and a shift, 33 of them with a
    // PReLU's slopes and shifts, and the input's exponent.
    assert_eq!(blob.len(), 50 * 3 + 33 * 2 + 1);
    let exponent = blob.get("input.exponent").expect("input exponent");
    assert_eq!(exponent.i32s().collect::<Vec<_>>(), [-6]);
    for (name, shape) in [
        ("conv_1.weight", [4usize, 32, 16]),
        ("conv_2_dw.weight", [4, 9, 16]),
        ("dconv_45_conv_sep.weight", [32, 128, 16]),
        ("conv_6dw7_7.weight", [32, 49, 16]),
        ("fc1.weight", [32, 512, 16]),
    ] {
        let entry = blob.get(name).expect(name);
        assert_eq!(
            (entry.data_type, entry.layout, entry.shape()),
            (DataType::I8, "N16HWC16", &shape[..]),
            "{name}"
        );
    }
    // The halves ESP-DL split, merged with a shift per group of each.
    let shifts: Vec<i32> = blob
        .get("dconv_45_conv_sep.shift")
        .expect("shifts")
        .i32s()
        .collect();
    assert_eq!(shifts.len(), 32);
    assert_ne!(shifts[0], shifts[31], "the two halves keep their scales");
    // The stem's 27 taps padded to 32: 320 zeros on top of the model's
    // weights.
    let total: usize = blob
        .entries()
        .filter(|entry| entry.name.ends_with(".weight"))
        .map(|entry| entry.element_count())
        .sum();
    assert_eq!(total - 320, 1_172_608, "weights of MFN_S8_V1");
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
    let mfn = Blob::parse(MFN_GOLDEN).expect("valid golden file");
    let input = mfn.get("input").expect("input");
    assert_eq!(
        (input.data_type, input.shape()),
        (DataType::I8, &[112usize, 112, 3][..])
    );
    let embedding = mfn.get("embedding").expect("embedding");
    assert_eq!(embedding.shape(), &[512]);
    assert!(
        embedding.i8s().any(|value| value != 0),
        "an embedding of zeros"
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
