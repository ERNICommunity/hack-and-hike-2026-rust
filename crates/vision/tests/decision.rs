//! The whole decision on real data: the fixture face through the board's
//! pipeline, enrolled in a gallery, against the impostor bank that
//! `facekit bank` built from 200 strangers.
//!
//! The other tests check the pieces. This one checks that they fit: the
//! recognizer's embeddings, the bank's `i8` form, and the
//! thresholds `facekit calibrate` chose.

mod common;

use hack_and_hike_vision::{
    align::CROP_SIZE,
    blob::Blob,
    gallery::{Embedding, Fusion, Gallery, ImpostorBank, Thresholds},
    image::RgbImage,
};

/// The impostor bank: 200 strangers from Labeled Faces in the Wild.
const IMPOSTORS: &[u8] = include_bytes!("../../../assets/models/impostors.fkb");

/// The embedding of the fixture face, through the recognizer as the
/// board runs it.
fn fixture_embedding() -> Embedding {
    let rgb = common::fixture_face();
    let crop = RgbImage::new(&rgb, CROP_SIZE, CROP_SIZE);
    Embedding::from_raw(&common::mfn_embedding(&crop))
}

/// The bank as the firmware reads it: `i8` values with one scale.
fn bank(bytes: &[u8]) -> (&[i8], f32) {
    let blob = Blob::parse(bytes).expect("valid bank file");
    let entry = blob.get("impostors").expect("the impostors tensor");
    assert_eq!(entry.shape(), &[200, 512], "200 strangers of 512 values");
    (entry.i8_slice(), entry.scale)
}

#[test]
fn the_bank_holds_unit_vectors_of_strangers() {
    let (values, scale) = bank(IMPOSTORS);
    let mut worst_length = 0.0f32;
    let mut best_pair = -1.0f32;
    let members: Vec<Vec<f32>> = values
        .chunks_exact(512)
        .map(|member| member.iter().map(|&q| f32::from(q) * scale).collect())
        .collect();
    for (index, member) in members.iter().enumerate() {
        let length = member.iter().map(|v| v * v).sum::<f32>().sqrt();
        worst_length = worst_length.max((length - 1.0).abs());
        // No two strangers may be the same person: that would make the
        // bank's own members score 1.0 against each other and would mean
        // `facekit bank` picked the same face twice.
        for other in &members[index + 1..] {
            best_pair = best_pair.max(member.iter().zip(other).map(|(a, b)| a * b).sum::<f32>());
        }
    }
    println!("bank: worst length error {worst_length:.4}, closest pair {best_pair:.3}");
    assert!(
        worst_length < 0.01,
        "the stored vectors are not unit length"
    );
    assert!(
        best_pair < 0.6,
        "two bank members look like the same person: {best_pair}"
    );
}

#[test]
fn the_fixture_face_is_recognized_and_a_stranger_is_not() {
    let face = fixture_embedding();

    let (values, scale) = bank(IMPOSTORS);
    let impostors = ImpostorBank::from_i8(values, scale);
    let thresholds = Thresholds::DEFAULT;

    // The face is not one of the strangers.
    let against_bank = impostors.best_similarity(&face);
    println!("the fixture face against the bank: {against_bank:.3}");
    assert!(
        against_bank < thresholds.accept,
        "the fixture face is in the bank: {against_bank}"
    );

    // Enroll it, then recognize it.
    let mut gallery = Gallery::new();
    {
        let person = gallery.enroll("Fixture").expect("an empty gallery");
        assert!(
            person.add_template(face),
            "an empty person takes a template"
        );
    }
    let verdict = gallery.match_probe(&face, &impostors, &thresholds);
    println!("verdict: {:?} at {:.3}", verdict.name(), verdict.score());
    assert_eq!(verdict.name(), Some("Fixture"));
    assert!(verdict.score() > 0.99, "a face matches itself");

    // Every stranger in the bank must be rejected by that gallery.
    let mut accepted = 0;
    let mut best_stranger = -1.0f32;
    for member in values.chunks_exact(512) {
        let raw: Vec<f32> = member.iter().map(|&q| f32::from(q) * scale).collect();
        let stranger = Embedding::from_raw(&raw);
        let verdict = gallery.match_probe(&stranger, &impostors, &thresholds);
        best_stranger = best_stranger.max(gallery.people()[0].best_similarity(&stranger));
        if verdict.is_known() {
            accepted += 1;
        }
    }
    println!("strangers accepted: {accepted} of 200 (closest scored {best_stranger:.3})");
    assert_eq!(accepted, 0, "a stranger was taken for the enrolled person");
}

#[test]
fn fusion_of_one_face_leaves_it_alone() {
    let face = fixture_embedding();

    // Three frames of the same still face: the average must be the face.
    let mut fusion = Fusion::new();
    for _ in 0..3 {
        fusion.push(face);
    }
    let fused = fusion.fused().expect("three frames are enough");
    let similarity = fused.similarity(&face);
    println!("three identical frames fuse to a vector at {similarity:.4}");
    assert!(
        similarity > 0.9999,
        "fusion moved a still face: {similarity}"
    );
}
