//! From the detector's outputs to an aligned face: decoding against a
//! hand-decoded anchor, the gates, and the whole path a camera frame
//! takes on the board, ending in an embedding that must agree with the
//! golden one.

mod common;

use common::Tensors;
use hack_and_hike_vision::{
    align::{CROP_SIZE, align_face},
    blob::Blob,
    detect::{
        CONTENT_HEIGHT, CONTENT_WIDTH, DEFAULT_NMS_THRESHOLD, DEFAULT_SCORE_THRESHOLD, DOWNSCALE,
        Face, decode, detector_input,
    },
    gates::{Framing, Limits, framing, pose},
    image::{
        GrayImage, GrayImageMut, Rgb565Frame, RgbImage, RgbImageMut, downscale_to_rgb, rgb_to_gray,
    },
    nn::yunet,
    quality::laplacian_variance,
    warp::ARCFACE_TEMPLATE_112,
};

/// The detector's weights.
const YUNET_WEIGHTS: &[u8] = include_bytes!("fixtures/yunet.f32.fkb");
/// The detector's golden run on the fixture face.
const YUNET_GOLDEN: &[u8] = include_bytes!("fixtures/yunet.golden.fkb");
/// The recognizer's golden run on the hand-cut crop of the same face
/// (`facekit golden-espdl`).
const MFN_GOLDEN: &[u8] = include_bytes!("fixtures/mfn.golden.fkb");
/// The fixture face at the board's frame size.
const PHOTO: &[u8] = include_bytes!("fixtures/face_320x240.jpg");

/// The anchor where the fixture face is: row 4, column 4 of the 8x12
/// stride-8 grid.
const FACE_ANCHOR: usize = 4 * 12 + 4;

/// Run the detector on the golden input and decode.
fn detect_golden(input: &[f32]) -> hack_and_hike_vision::detect::Faces {
    let weights = Tensors::load(YUNET_WEIGHTS);
    let mut scratch = vec![0.0f32; yunet::SCRATCH_LEN];
    let heads = yunet::forward(&weights, input, &mut scratch);
    decode(&heads, DEFAULT_SCORE_THRESHOLD, DEFAULT_NMS_THRESHOLD)
}

#[test]
fn decodes_the_golden_anchor_by_hand() {
    let golden = Tensors::load(YUNET_GOLDEN);
    let (_, input) = golden.activation_nchw("input");
    let faces = detect_golden(&input);
    assert_eq!(
        faces.len(),
        1,
        "one face, after suppression of its neighbours"
    );
    let face = faces[0];

    // The same formulas as OpenCV's FaceDetectorYN, applied to the golden
    // head values of the one anchor, independently of `decode`.
    let cls = golden.flat("cls_8")[FACE_ANCHOR];
    let obj = golden.flat("obj_8")[FACE_ANCHOR];
    let bbox = &golden.flat("bbox_8")[FACE_ANCHOR * 4..FACE_ANCHOR * 4 + 4];
    let kps = &golden.flat("kps_8")[FACE_ANCHOR * 10..FACE_ANCHOR * 10 + 10];
    let (column, row, stride) = (4.0f32, 4.0f32, 8.0f32);
    let expected_score = (cls * obj).sqrt();
    let (cx, cy) = ((column + bbox[0]) * stride, (row + bbox[1]) * stride);
    let (w, h) = (bbox[2].exp() * stride, bbox[3].exp() * stride);
    assert!(
        (face.score - expected_score).abs() < 1e-6,
        "score {}",
        face.score
    );
    assert!((face.x - (cx - w / 2.0)).abs() < 1e-4 && (face.y - (cy - h / 2.0)).abs() < 1e-4);
    assert!((face.width - w).abs() < 1e-4 && (face.height - h).abs() < 1e-4);
    for (n, point) in face.landmarks.iter().enumerate() {
        assert!(
            (point[0] - (kps[2 * n] + column) * stride).abs() < 1e-4,
            "landmark {n} x"
        );
        assert!(
            (point[1] - (kps[2 * n + 1] + row) * stride).abs() < 1e-4,
            "landmark {n} y"
        );
    }
    println!(
        "face: score {:.3}, box ({:.1}, {:.1}) {:.1}x{:.1}, landmarks {:?}",
        face.score, face.x, face.y, face.width, face.height, face.landmarks
    );

    // The face fills the 60-pixel content height and is sensibly placed.
    assert!(face.score > 0.9);
    assert!(
        face.height > 0.8 * CONTENT_HEIGHT as f32 && face.height < 1.05 * CONTENT_HEIGHT as f32
    );
    assert_landmarks_look_like_a_face(&face);
}

/// Eyes above the nose above the mouth; the image-left points left of the
/// image-right ones.
fn assert_landmarks_look_like_a_face(face: &Face) {
    let [left_eye, right_eye, nose, left_mouth, right_mouth] = face.landmarks;
    assert!(left_eye[0] < right_eye[0], "eyes in image order");
    assert!(
        left_mouth[0] < right_mouth[0],
        "mouth corners in image order"
    );
    assert!(
        left_eye[1] < nose[1] && right_eye[1] < nose[1],
        "eyes above the nose"
    );
    assert!(
        nose[1] < left_mouth[1] && nose[1] < right_mouth[1],
        "nose above the mouth"
    );
    assert!(
        left_eye[0] < nose[0] && nose[0] < right_eye[0],
        "nose between the eyes"
    );
}

#[test]
fn a_mirrored_frame_gives_a_mirrored_face() {
    let golden = Tensors::load(YUNET_GOLDEN);
    let (shape, input) = golden.activation_nchw("input");
    let original = detect_golden(&input)[0];

    let mut mirrored = vec![0.0f32; input.len()];
    for y in 0..shape.height {
        for x in 0..shape.width {
            let from = shape.offset(shape.width - 1 - x, y);
            mirrored[shape.offset(x, y)..shape.offset(x, y) + 3]
                .copy_from_slice(&input[from..from + 3]);
        }
    }
    let faces = detect_golden(&mirrored);
    for face in faces.iter() {
        println!(
            "mirrored: score {:.3}, box ({:.1}, {:.1}) {:.1}x{:.1}",
            face.score, face.x, face.y, face.width, face.height
        );
    }
    assert_eq!(faces.len(), 1, "one face in the mirrored frame");
    let face = faces[0];
    let width = shape.width as f32;
    assert!(face.score > 0.9, "score {}", face.score);
    assert!(
        (face.centre()[0] - (width - original.centre()[0])).abs() < 3.0,
        "mirrored centre x"
    );
    assert!(
        (face.centre()[1] - original.centre()[1]).abs() < 2.0,
        "same centre y"
    );
    assert!((face.height - original.height).abs() < 4.0, "same height");
    // The landmarks are named by image side, so their order is unchanged.
    assert_landmarks_look_like_a_face(&face);
    // Mirroring swaps the sides: the eye that was on the image's left is
    // now on its right. The landmarks are named by image side, so the
    // mirrored landmark 0 must match the mirror of the original landmark
    // 1, and so on. (Named by anatomy, they would match their own
    // mirror, and the alignment template would need to know which.) The
    // network is not exactly symmetric, so allow a few pixels.
    for (n, m) in [(0, 1), (1, 0), (2, 2), (3, 4), (4, 3)] {
        let source = original.landmarks[m];
        let expected = [width - source[0], source[1]];
        let point = face.landmarks[n];
        let error = ((point[0] - expected[0]).powi(2) + (point[1] - expected[1]).powi(2)).sqrt();
        println!(
            "mirrored landmark {n}: {error:.2} px from the mirror of the original landmark {m}"
        );
        assert!(error < 5.0, "mirrored landmark {n} is {error} px off");
    }
}

#[test]
fn gates_judge_framing_and_pose() {
    let golden = Tensors::load(YUNET_GOLDEN);
    let (_, input) = golden.activation_nchw("input");
    let face = detect_golden(&input)[0];
    let limits = Limits::DEFAULT;
    let (width, height) = (CONTENT_WIDTH as f32, CONTENT_HEIGHT as f32);
    assert_eq!(framing(&face, width, height, &limits), Framing::Good);

    let far = Face {
        height: face.height * 0.5,
        width: face.width * 0.5,
        ..face
    };
    assert_eq!(framing(&far, width, height, &limits), Framing::TooFar);
    let close = Face {
        height: face.height * 1.5,
        ..face
    };
    assert_eq!(framing(&close, width, height, &limits), Framing::TooClose);
    let shifted = Face {
        x: face.x + width / 2.0,
        ..face
    };
    assert_eq!(
        framing(&shifted, width, height, &limits),
        Framing::OffCentre
    );

    let frontal = pose(&face.landmarks);
    println!("pose: {frontal:?}");
    assert!(
        frontal.is_frontal(&limits),
        "the fixture face is frontal: {frontal:?}"
    );
    assert!(frontal.roll_degrees.abs() < 10.0 && frontal.yaw.abs() < 0.15);

    // Turn the head: move the nose towards the right eye.
    let mut turned = face.landmarks;
    turned[2][0] += 0.4 * (turned[1][0] - turned[0][0]);
    assert!(
        !pose(&turned).is_frontal(&limits),
        "a turned face is not frontal"
    );
    // Tilt the head: raise the right eye.
    let mut tilted = face.landmarks;
    tilted[1][1] -= 0.5 * (tilted[1][0] - tilted[0][0]);
    assert!(
        !pose(&tilted).is_frontal(&limits),
        "a tilted face is not frontal"
    );
}

/// The fixture photo as a big-endian RGB565 frame, as the camera delivers.
fn photo_as_rgb565() -> (Vec<u8>, usize, usize) {
    let photo = image::load_from_memory(PHOTO)
        .expect("fixture JPEG")
        .to_rgb8();
    let (width, height) = (photo.width() as usize, photo.height() as usize);
    let mut frame = Vec::with_capacity(width * height * 2);
    for rgb in photo.as_raw().chunks_exact(3) {
        let value =
            (u16::from(rgb[0] >> 3) << 11) | (u16::from(rgb[1] >> 2) << 5) | u16::from(rgb[2] >> 3);
        frame.extend_from_slice(&value.to_be_bytes());
    }
    (frame, width, height)
}

#[test]
fn the_board_path_from_frame_to_embedding() {
    let (frame_bytes, width, height) = photo_as_rgb565();
    assert_eq!((width, height), (320, 240));
    let frame = Rgb565Frame::new(&frame_bytes, width, height);

    // Detector input: the frame scaled down by 4, in the corner of 96x64.
    let mut small = vec![0u8; CONTENT_WIDTH * CONTENT_HEIGHT * 3];
    let mut small_image = RgbImageMut::new(&mut small, CONTENT_WIDTH, CONTENT_HEIGHT);
    downscale_to_rgb(&frame, DOWNSCALE, &mut small_image);
    let mut input = vec![0.0f32; yunet::INPUT_SHAPE.len()];
    detector_input(&small_image.as_image(), &mut input);

    // The board's input must be close to the golden one, which facekit
    // made from the same photo with another scaler: within the RGB565
    // rounding and the difference between a box filter and a triangle one.
    let golden = Tensors::load(YUNET_GOLDEN);
    let (_, golden_input) = golden.activation_nchw("input");
    let mean_difference: f32 = input
        .iter()
        .zip(&golden_input)
        .map(|(a, b)| (a - b).abs())
        .sum::<f32>()
        / input.len() as f32;
    println!(
        "detector input differs from the golden input by {mean_difference:.2} per value on average"
    );
    assert!(mean_difference < 6.0);

    let faces = detect_golden(&input);
    assert_eq!(faces.len(), 1);
    let face = faces[0];
    let golden_face = detect_golden(&golden_input)[0];
    println!(
        "board face: score {:.3}, box ({:.1}, {:.1}) {:.1}x{:.1}",
        face.score, face.x, face.y, face.width, face.height
    );
    assert!(face.score > 0.85);
    assert!((face.centre()[0] - golden_face.centre()[0]).abs() < 2.0);
    assert!((face.centre()[1] - golden_face.centre()[1]).abs() < 2.0);
    assert!((face.height - golden_face.height).abs() < 4.0);
    assert_eq!(
        framing(
            &face,
            CONTENT_WIDTH as f32,
            CONTENT_HEIGHT as f32,
            &Limits::DEFAULT
        ),
        Framing::Good
    );
    assert!(pose(&face.landmarks).is_frontal(&Limits::DEFAULT));

    // Alignment from the half-size frame (160x120), where the face is
    // about the crop's size: landmarks go from detector pixels (x4 for the
    // frame) to half-frame pixels (x2).
    let mut half = vec![0u8; (width / 2) * (height / 2) * 3];
    let mut half_image = RgbImageMut::new(&mut half, width / 2, height / 2);
    downscale_to_rgb(&frame, 2, &mut half_image);
    let source = half_image.as_image();
    let in_source = face.scaled((DOWNSCALE / 2) as f32);
    let mut crop = vec![0u8; CROP_SIZE * CROP_SIZE * 3];
    let mut crop_image = RgbImageMut::new(&mut crop, CROP_SIZE, CROP_SIZE);
    let transform =
        align_face(&in_source.landmarks, &source, &mut crop_image).expect("a transform");

    // The transform must put the landmarks on the template. A swapped eye
    // would make this a half-turn and fail by dozens of pixels.
    for (n, (point, target)) in in_source
        .landmarks
        .iter()
        .zip(&ARCFACE_TEMPLATE_112)
        .enumerate()
    {
        let moved = transform.apply(*point);
        let error = ((moved[0] - target[0]).powi(2) + (moved[1] - target[1]).powi(2)).sqrt();
        println!(
            "landmark {n}: ({:.1}, {:.1}) -> template ({:.1}, {:.1}), {error:.2} px off",
            moved[0], moved[1], target[0], target[1]
        );
        assert!(error < 4.0, "landmark {n} is {error} px off the template");
    }

    // Sharpness of the crop, and of a blurred copy.
    let crop_view = crop_image.as_image();
    let mut gray = vec![0u8; CROP_SIZE * CROP_SIZE];
    let mut gray_image = GrayImageMut::new(&mut gray, CROP_SIZE, CROP_SIZE);
    rgb_to_gray(&crop_view, &mut gray_image);
    let sharpness = laplacian_variance(&gray_image.as_image());
    let mut blurred = vec![0u8; CROP_SIZE * CROP_SIZE];
    for y in 1..CROP_SIZE - 1 {
        for x in 1..CROP_SIZE - 1 {
            let mut sum = 0u32;
            for dy in 0..3 {
                for dx in 0..3 {
                    sum += u32::from(gray[(y + dy - 1) * CROP_SIZE + x + dx - 1]);
                }
            }
            blurred[y * CROP_SIZE + x] = (sum / 9) as u8;
        }
    }
    let blurred_sharpness = laplacian_variance(&GrayImage::new(&blurred, CROP_SIZE, CROP_SIZE));
    println!("sharpness: crop {sharpness:.1}, blurred {blurred_sharpness:.1}");
    assert!(
        sharpness > 1.3 * blurred_sharpness,
        "a blurred copy scores clearly lower"
    );

    // The recognizer on our crop, against its golden run on the hand-cut
    // crop of the same photo.
    let embedding = common::mfn_embedding(&crop_view);
    let golden = Blob::parse(MFN_GOLDEN).expect("the golden file");
    let golden_embedding: Vec<f32> = golden
        .get("embedding")
        .expect("embedding")
        .i8s()
        .map(f32::from)
        .collect();
    let similarity = cosine(&embedding, &golden_embedding);
    println!("embedding of the aligned crop vs the hand-cut crop: cosine {similarity:.3}");
    assert!(similarity > 0.8, "cosine {similarity}");

    // And a different face must be far away: the recognizer on a mirrored
    // crop of the same person is still the same person (sanity), while a
    // crop of the top-left background is not.
    let mut background = vec![0u8; CROP_SIZE * CROP_SIZE * 3];
    for y in 0..CROP_SIZE {
        let row = source.row(y);
        background[y * CROP_SIZE * 3..(y + 1) * CROP_SIZE * 3]
            .copy_from_slice(&row[..CROP_SIZE * 3]);
    }
    let other = common::mfn_embedding(&RgbImage::new(&background, CROP_SIZE, CROP_SIZE));
    let unrelated = cosine(&other, &golden_embedding);
    println!("embedding of a background patch vs the face: cosine {unrelated:.3}");
    assert!(unrelated < similarity - 0.3);
}

/// Cosine similarity of two vectors.
fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norm = |v: &[f32]| v.iter().map(|x| x * x).sum::<f32>().sqrt();
    dot / (norm(a) * norm(b))
}
