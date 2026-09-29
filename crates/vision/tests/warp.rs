//! Similarity fitting and bilinear warping on synthetic images.

use hack_and_hike_vision::image::{GrayImage, GrayImageMut, RgbImage, RgbImageMut};
use hack_and_hike_vision::warp::{ARCFACE_TEMPLATE_112, Similarity, fit, footprint, warp};

/// A similarity that rotates by `degrees`, scales by `scale` and then moves
/// by `(tx, ty)`.
fn similarity(degrees: f32, scale: f32, tx: f32, ty: f32) -> Similarity {
    let radians = degrees.to_radians();
    Similarity {
        a: scale * radians.cos(),
        b: scale * radians.sin(),
        tx,
        ty,
    }
}

/// A `size` x `size` gray image with a smooth diagonal gradient: pixel
/// `(x, y)` is `2 * (x + y)`, at most 252 for a size of 64.
fn gradient(size: usize) -> Vec<u8> {
    let mut data = Vec::with_capacity(size * size);
    for y in 0..size {
        for x in 0..size {
            data.push((2 * (x + y)) as u8);
        }
    }
    data
}

#[test]
fn fit_recovers_a_known_similarity() {
    let expected = similarity(20.0, 1.7, 30.0, -12.0);
    let moved: Vec<[f32; 2]> = ARCFACE_TEMPLATE_112
        .iter()
        .map(|point| expected.apply(*point))
        .collect();
    let fitted = fit(&ARCFACE_TEMPLATE_112, &moved).expect("five distinct points fit");
    assert!((fitted.a - expected.a).abs() < 1e-4, "a {fitted:?}");
    assert!((fitted.b - expected.b).abs() < 1e-4, "b {fitted:?}");
    assert!((fitted.tx - expected.tx).abs() < 1e-4, "tx {fitted:?}");
    assert!((fitted.ty - expected.ty).abs() < 1e-4, "ty {fitted:?}");
}

#[test]
fn inverse_undoes_the_transform() {
    let forward = similarity(20.0, 1.7, 30.0, -12.0);
    let inverse = forward.inverse().expect("a scale of 1.7 is invertible");
    let round_trip = forward.then(&inverse);
    let other_way = inverse.then(&forward);
    for point in [
        [0.0, 0.0],
        [112.0, 0.0],
        [0.0, 112.0],
        [56.0, 71.5],
        [-10.0, 33.0],
    ] {
        for transform in [&round_trip, &other_way] {
            let [x, y] = transform.apply(point);
            assert!((x - point[0]).abs() < 1e-5, "{point:?} -> {x}");
            assert!((y - point[1]).abs() < 1e-5, "{point:?} -> {y}");
        }
    }
}

#[test]
fn then_applies_the_transforms_in_order() {
    let first = similarity(90.0, 2.0, 1.0, 0.0);
    let second = similarity(0.0, 1.0, 0.0, 5.0);
    let both = first.then(&second);
    let point = [1.0, 0.0];
    let expected = second.apply(first.apply(point));
    let [x, y] = both.apply(point);
    assert!((x - expected[0]).abs() < 1e-5 && (y - expected[1]).abs() < 1e-5);
    // Rotating (1, 0) by 90 degrees and scaling by 2 gives (0, 2), plus
    // the translations (1, 0) and (0, 5): (1, 7).
    assert!(
        (x - 1.0).abs() < 1e-5 && (y - 7.0).abs() < 1e-5,
        "({x}, {y})"
    );
}

#[test]
fn identity_and_degenerate_inverses() {
    assert_eq!(Similarity::IDENTITY.apply([3.0, -4.5]), [3.0, -4.5]);
    assert_eq!(Similarity::IDENTITY.inverse(), Some(Similarity::IDENTITY));
    let flat = Similarity {
        a: 0.0,
        b: 0.0,
        tx: 1.0,
        ty: 1.0,
    };
    assert_eq!(flat.inverse(), None);
    let broken = Similarity {
        a: f32::NAN,
        b: 0.0,
        tx: 0.0,
        ty: 0.0,
    };
    assert_eq!(broken.inverse(), None);
}

#[test]
fn fit_needs_two_distinct_points() {
    assert_eq!(fit(&[], &[]), None);
    assert_eq!(fit(&[[1.0, 2.0]], &[[3.0, 4.0]]), None);
    assert_eq!(
        fit(&[[1.0, 2.0], [1.0, 2.0]], &[[3.0, 4.0], [5.0, 6.0]]),
        None
    );
    assert_eq!(
        fit(&[[1.0, 2.0], [3.0, 4.0]], &[[3.0, 4.0]]),
        None,
        "different lengths"
    );
    let two = fit(&[[0.0, 0.0], [1.0, 0.0]], &[[0.0, 0.0], [0.0, 2.0]]).expect("two points fit");
    // (1, 0) to (0, 2) is a rotation by 90 degrees and a scale of 2.
    assert!(
        (two.a - 0.0).abs() < 1e-6 && (two.b - 2.0).abs() < 1e-6,
        "{two:?}"
    );
}

#[test]
fn warp_with_the_identity_copies_the_image() {
    let data = gradient(64);
    let src = GrayImage::new(&data, 64, 64);
    let mut out = vec![0u8; 64 * 64];
    let mut dst = GrayImageMut::new(&mut out, 64, 64);
    warp(&src, &Similarity::IDENTITY, &mut dst);
    assert_eq!(out, data);
}

#[test]
fn warp_there_and_back_keeps_the_interior() {
    // `forward` maps the 64x64 source into a 160x160 intermediate image,
    // rotated by 10 degrees and enlarged by 1.3. All four corners of the
    // source land inside the intermediate image.
    let forward = similarity(10.0, 1.3, 20.0, 10.0);
    let backward = forward.inverse().expect("invertible");

    let data = gradient(64);
    let src = GrayImage::new(&data, 64, 64);
    let mut middle = vec![0u8; 160 * 160];
    let mut middle_dst = GrayImageMut::new(&mut middle, 160, 160);
    // The destination is the intermediate image, so the transform goes
    // from the intermediate coordinates back to the source: `backward`.
    warp(&src, &backward, &mut middle_dst);

    let middle_src = GrayImage::new(&middle, 160, 160);
    let mut out = vec![0u8; 64 * 64];
    let mut dst = GrayImageMut::new(&mut out, 64, 64);
    warp(&middle_src, &forward, &mut dst);

    // The black border of the intermediate image bleeds into the outermost
    // source pixels, so compare the interior only.
    let mut total_error = 0u32;
    let mut count = 0u32;
    for y in 4..60 {
        for x in 4..60 {
            let original = i32::from(data[y * 64 + x]);
            let restored = i32::from(out[y * 64 + x]);
            total_error += (original - restored).unsigned_abs();
            count += 1;
        }
    }
    let mean_error = total_error as f32 / count as f32;
    assert!(mean_error < 4.0, "mean absolute error {mean_error}");
}

#[test]
fn warp_outside_the_source_is_black() {
    let data = gradient(16);
    let src = GrayImage::new(&data, 16, 16);
    let far_away = Similarity {
        a: 1.0,
        b: 0.0,
        tx: 1000.0,
        ty: 1000.0,
    };
    let mut out = vec![7u8; 16 * 16];
    let mut dst = GrayImageMut::new(&mut out, 16, 16);
    warp(&src, &far_away, &mut dst);
    assert!(dst.data().iter().all(|&value| value == 0));

    // Partly outside: a shift by half a pixel needs the pixel to the right,
    // so the last column has no complete neighbourhood and is black, the
    // rest is not.
    let half = Similarity {
        a: 1.0,
        b: 0.0,
        tx: 0.5,
        ty: 0.0,
    };
    warp(&src, &half, &mut dst);
    for y in 0..16 {
        assert_eq!(dst.pixel(15, y), [0], "last column of row {y}");
        assert_eq!(dst.pixel(1, y), [(2 * y + 3) as u8], "column 1 of row {y}");
    }
}

#[test]
fn warp_keeps_the_channels_apart() {
    let mut data = Vec::with_capacity(32 * 32 * 3);
    for y in 0..32u8 {
        for x in 0..32u8 {
            data.extend_from_slice(&[x, y, 7]);
        }
    }
    let src = RgbImage::new(&data, 32, 32);
    let mut out = vec![0u8; 32 * 32 * 3];
    let mut dst = RgbImageMut::new(&mut out, 32, 32);
    warp(&src, &Similarity::IDENTITY, &mut dst);
    assert_eq!(dst.data(), &data[..]);
    // A shift by a whole pixel moves the red gradient by one and leaves
    // green and blue unchanged.
    let shifted = Similarity {
        a: 1.0,
        b: 0.0,
        tx: 1.0,
        ty: 0.0,
    };
    warp(&src, &shifted, &mut dst);
    assert_eq!(dst.pixel(3, 5), [4, 5, 7]);
    assert_eq!(dst.pixel(31, 5), [0, 0, 0]);
}

#[test]
fn the_footprint_holds_every_pixel_the_warp_reads() {
    // A 64x48 RGB source of values that look random. For each transform,
    // a second source keeps the footprint and has every other pixel
    // replaced by garbage: the warps of the two must be equal byte for
    // byte. Rotations, scales and shifts that put the crop inside the
    // source, across its edges and outside it.
    let (width, height) = (64usize, 48usize);
    let mut state = 12_345u32;
    let source: Vec<u8> = (0..width * height * 3)
        .map(|_| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (state >> 24) as u8
        })
        .collect();
    let mut checked = 0;
    for degrees in [-37.0f32, -8.0, 0.0, 3.5, 21.0, 90.0, 180.0] {
        for scale in [0.35f32, 0.5, 0.71, 1.0, 1.9] {
            for (tx, ty) in [(5.0f32, 3.0f32), (20.3, 11.7), (50.0, 40.0), (-9.0, 30.0)] {
                let transform = similarity(degrees, scale, tx, ty);
                let (columns, rows) =
                    footprint(&transform, 28, 28, width, height).expect("a finite transform");
                assert!(columns.end <= width && rows.end <= height);
                let mut poisoned = vec![0xA5u8; source.len()];
                for y in rows.clone() {
                    let range = (y * width + columns.start) * 3..(y * width + columns.end) * 3;
                    poisoned[range.clone()].copy_from_slice(&source[range]);
                }
                let mut expected = vec![0u8; 28 * 28 * 3];
                let mut actual = vec![0u8; 28 * 28 * 3];
                warp(
                    &RgbImage::new(&source, width, height),
                    &transform,
                    &mut RgbImageMut::new(&mut expected, 28, 28),
                );
                warp(
                    &RgbImage::new(&poisoned, width, height),
                    &transform,
                    &mut RgbImageMut::new(&mut actual, 28, 28),
                );
                assert_eq!(
                    actual, expected,
                    "{degrees} degrees, scale {scale}, shift ({tx}, {ty})"
                );
                checked += 1;
            }
        }
    }
    assert_eq!(checked, 7 * 5 * 4);
}

#[test]
fn a_transform_that_is_not_finite_has_no_footprint() {
    let broken = Similarity {
        a: f32::NAN,
        ..Similarity::IDENTITY
    };
    assert_eq!(footprint(&broken, 4, 4, 10, 10), None);
    assert_eq!(
        footprint(&Similarity::IDENTITY, 0, 4, 10, 10),
        Some((0..0, 0..0))
    );
    // The identity reads the destination's own pixels, and one more on
    // every side that is inside the source.
    assert_eq!(
        footprint(&Similarity::IDENTITY, 4, 3, 10, 10),
        Some((0..6, 0..5))
    );
}
