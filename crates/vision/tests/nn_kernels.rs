//! The `f32` kernels on small inputs with known answers.

use hack_and_hike_vision::nn::{
    Activation, Shape, add_scaled, conv2d, depthwise, gelu, global_average, l2_normalize_rows,
    layer_norm, linear, max_pool_2x2, sigmoid, softmax_rows, upsample_2x_add,
};

/// A 4x4 single-channel ramp: pixel (x, y) has value `y * 4 + x`.
fn ramp() -> Vec<f32> {
    (0..16).map(|value| value as f32).collect()
}

#[test]
fn conv2d_matches_hand_computation() {
    // One input channel, one output channel, a 3x3 kernel that takes the
    // centre minus the pixel to the right, padding 1, stride 1.
    let input = ramp();
    let shape = Shape::new(4, 4, 1);
    let mut weight = [0.0f32; 9];
    weight[4] = 1.0; // centre
    weight[5] = -1.0; // right neighbour
    let mut output = vec![0.0; 16];
    let out = conv2d(
        &input,
        shape,
        &weight,
        &[0.5],
        3,
        1,
        1,
        Activation::None,
        &mut output,
    );
    assert_eq!(out, shape);
    // Inside a row: value - (value + 1) + 0.5 = -0.5. On the right edge the
    // neighbour is padding (0): value + 0.5.
    assert_eq!(output[0], -0.5);
    assert_eq!(output[3], 3.5);
    assert_eq!(output[15], 15.5);

    // Relu clamps the negative results.
    conv2d(
        &input,
        shape,
        &weight,
        &[0.5],
        3,
        1,
        1,
        Activation::Relu,
        &mut output,
    );
    assert_eq!(output[0], 0.0);
    assert_eq!(output[3], 3.5);

    // Stride 2 with a 2x2 kernel of ones and no padding sums each block.
    let ones = [1.0f32; 4];
    let out = conv2d(
        &input,
        shape,
        &ones,
        &[0.0],
        2,
        2,
        0,
        Activation::None,
        &mut output,
    );
    assert_eq!(out, Shape::new(2, 2, 1));
    assert_eq!(
        &output[..4],
        &[
            0.0 + 1.0 + 4.0 + 5.0,
            2.0 + 3.0 + 6.0 + 7.0,
            8.0 + 9.0 + 12.0 + 13.0,
            10.0 + 11.0 + 14.0 + 15.0
        ]
    );
}

#[test]
fn conv2d_mixes_channels_in_ohwi_order() {
    // 1x1 image, 2 input channels, 3 output channels: a plain matrix.
    let input = [1.0f32, 10.0];
    let shape = Shape::new(1, 1, 2);
    // OHWI with a 1x1 kernel is [O][I].
    let weight = [1.0f32, 0.0, 0.0, 1.0, 2.0, 3.0];
    let mut output = [0.0f32; 3];
    conv2d(
        &input,
        shape,
        &weight,
        &[0.0, 0.0, 100.0],
        1,
        1,
        0,
        Activation::None,
        &mut output,
    );
    assert_eq!(output, [1.0, 10.0, 132.0]);
}

#[test]
fn depthwise_keeps_channels_apart() {
    // 3x3 image, 2 channels: channel 0 is a ramp, channel 1 is constant 1.
    let shape = Shape::new(3, 3, 2);
    let mut input = vec![0.0f32; shape.len()];
    for y in 0..3 {
        for x in 0..3 {
            input[shape.offset(x, y)] = (y * 3 + x) as f32;
            input[shape.offset(x, y) + 1] = 1.0;
        }
    }
    // HWC weights: channel 0 takes the centre, channel 1 sums the 3x3 window.
    let mut weight = vec![0.0f32; 9 * 2];
    weight[4 * 2] = 1.0;
    for tap in 0..9 {
        weight[tap * 2 + 1] = 1.0;
    }
    let mut output = vec![0.0; shape.len()];
    let out = depthwise(
        &input,
        shape,
        &weight,
        &[0.0, 0.0],
        3,
        1,
        1,
        Activation::None,
        &mut output,
    );
    assert_eq!(out, shape);
    assert_eq!(output[shape.offset(1, 1)], 4.0);
    // The window of the centre pixel is fully inside: 9 ones. A corner sees
    // 4 ones and 5 zeros of padding.
    assert_eq!(output[shape.offset(1, 1) + 1], 9.0);
    assert_eq!(output[shape.offset(0, 0) + 1], 4.0);
}

#[test]
fn linear_applies_weight_rows_and_bias() {
    let input = [1.0f32, 2.0, 3.0, 4.0]; // two rows of two features
    let weight = [1.0f32, 1.0, 1.0, -1.0, 0.0, 2.0]; // three outputs: sum, difference, twice the second
    let bias = [0.0f32, 0.0, 0.5];
    let mut output = [0.0f32; 6];
    let rows = linear(
        &input,
        2,
        &weight,
        Some(&bias),
        3,
        Activation::None,
        &mut output,
    );
    assert_eq!(rows, 2);
    assert_eq!(output, [3.0, -1.0, 4.5, 7.0, -1.0, 8.5]);
    linear(&input, 2, &weight, None, 3, Activation::Relu, &mut output);
    assert_eq!(output, [3.0, 0.0, 4.0, 7.0, 0.0, 8.0]);
}

#[test]
fn layer_norm_standardizes_each_row() {
    let mut data = [1.0f32, 3.0, 2.0, 2.0];
    layer_norm(&mut data, 2, &[1.0, 1.0], &[0.0, 0.0], 1e-6);
    assert!((data[0] + 1.0).abs() < 1e-5 && (data[1] - 1.0).abs() < 1e-5);
    // A constant row has zero variance: epsilon keeps it finite (0).
    assert_eq!(&data[2..], &[0.0, 0.0]);
    // Weight and bias apply after standardization.
    let mut data = [1.0f32, 3.0];
    layer_norm(&mut data, 2, &[2.0, 3.0], &[10.0, 20.0], 0.0);
    assert!((data[0] - 8.0).abs() < 1e-5 && (data[1] - 23.0).abs() < 1e-5);
}

#[test]
fn pointwise_functions_match_known_values() {
    let mut data = [0.0f32, 1.0, -1.0, 3.0];
    gelu(&mut data);
    // Reference values of exact GELU.
    assert!((data[0]).abs() < 1e-6);
    assert!((data[1] - 0.841_345).abs() < 1e-5);
    assert!((data[2] + 0.158_655).abs() < 1e-5);
    assert!((data[3] - 2.995_95).abs() < 1e-5);

    let mut data = [0.0f32, 2.0, -2.0];
    sigmoid(&mut data);
    assert!((data[0] - 0.5).abs() < 1e-6);
    assert!((data[1] - 0.880_797).abs() < 1e-5);
    assert!((data[2] - 0.119_203).abs() < 1e-5);

    let mut data = [1.0f32, 2.0, 3.0, 0.0, 0.0, 0.0];
    softmax_rows(&mut data, 3);
    assert!((data[..3].iter().sum::<f32>() - 1.0).abs() < 1e-6);
    assert!((data[2] - 0.665_241).abs() < 1e-5);
    assert!(
        data[3..]
            .iter()
            .all(|value| (value - 0.333_333).abs() < 1e-5)
    );

    let mut data = [3.0f32, 4.0, 0.0, 0.0];
    l2_normalize_rows(&mut data, 2, 1e-12);
    assert_eq!(&data[..2], &[0.6, 0.8]);
    assert_eq!(&data[2..], &[0.0, 0.0]);
}

#[test]
fn pooling_upsampling_and_averaging() {
    let input = ramp();
    let shape = Shape::new(4, 4, 1);
    let mut pooled = vec![0.0; 4];
    let out = max_pool_2x2(&input, shape, &mut pooled);
    assert_eq!(out, Shape::new(2, 2, 1));
    assert_eq!(pooled, [5.0, 7.0, 13.0, 15.0]);

    // Odd sizes drop the last row and column.
    let odd = Shape::new(3, 3, 1);
    let mut one = [0.0f32; 1];
    let out = max_pool_2x2(&input[..9], odd, &mut one);
    assert_eq!(out, Shape::new(1, 1, 1));
    assert_eq!(one[0], 4.0);

    let mut target = vec![1.0; 16];
    upsample_2x_add(&pooled[..4], Shape::new(2, 2, 1), &mut target, shape);
    // Each pooled value lands on its 2x2 block, plus the 1 that was there.
    let expected: Vec<f32> = [
        5.0, 5.0, 7.0, 7.0, 5.0, 5.0, 7.0, 7.0, 13.0, 13.0, 15.0, 15.0, 13.0, 13.0, 15.0, 15.0,
    ]
    .iter()
    .map(|value| value + 1.0)
    .collect();
    assert_eq!(target, expected);

    let mut mean = [0.0f32; 1];
    global_average(&input, shape, &mut mean);
    assert_eq!(mean[0], 7.5);

    let mut target = [1.0f32, 2.0, 3.0, 4.0];
    add_scaled(&mut target, &[1.0, 1.0, 1.0, 1.0], &[10.0, 100.0]);
    assert_eq!(target, [11.0, 102.0, 13.0, 104.0]);
}
