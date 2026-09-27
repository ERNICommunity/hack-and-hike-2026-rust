//! The detector's integer kernels against plain loops, bit for bit.
//!
//! The kernels in `nn::quant` compute several outputs per pass over the
//! input and group a convolution's reads into runs. Their sums are exact
//! integers, so with an `f32` output they must give the same bits as the
//! simplest possible loop, whatever the shape, padding, stride or channel
//! count; with an `i16` output they requantize the exact sum once, which
//! lands within one step of the `f32` value quantized.

use hack_and_hike_vision::nn::{
    Activation, Shape,
    pack::{pack_rows, packed_index},
    quant::{self, Output, QWeight, Quant, Requant, round_to_int, simd::decode_lanes},
};

/// A small deterministic generator (an LCG), enough to vary the inputs.
struct Random(u64);

impl Random {
    fn next(&mut self) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) as u32
    }

    fn below(&mut self, bound: usize) -> usize {
        self.next() as usize % bound
    }

    fn i8(&mut self) -> i8 {
        (self.next() % 256) as u8 as i8
    }

    fn i16(&mut self) -> i16 {
        (self.next() % 65536) as u16 as i16
    }

    fn unit(&mut self) -> f32 {
        self.next() as f32 / u32::MAX as f32
    }
}

/// The output size of a convolution.
fn output_size(size: usize, kernel: usize, padding: usize, stride: usize) -> usize {
    (size + 2 * padding - kernel) / stride + 1
}

/// The plain full convolution: one `i64` sum per output.
fn reference_conv2d(
    input: &[i16],
    in_scale: f32,
    shape: Shape,
    weight: &QWeight<'_>,
    kernel: usize,
    stride: usize,
    padding: usize,
) -> (Shape, Vec<f32>) {
    let out_channels = weight.bias.len();
    let out = Shape::new(
        output_size(shape.height, kernel, padding, stride),
        output_size(shape.width, kernel, padding, stride),
        out_channels,
    );
    let taps = kernel * kernel * shape.channels;
    let mut result = vec![0.0f32; out.len()];
    for oy in 0..out.height {
        for ox in 0..out.width {
            for o in 0..out_channels {
                let mut sum = 0i64;
                for ky in 0..kernel {
                    for kx in 0..kernel {
                        let iy = (oy * stride + ky) as isize - padding as isize;
                        let ix = (ox * stride + kx) as isize - padding as isize;
                        if iy < 0
                            || ix < 0
                            || iy >= shape.height as isize
                            || ix >= shape.width as isize
                        {
                            continue;
                        }
                        for c in 0..shape.channels {
                            let value = input[shape.offset(ix as usize, iy as usize) + c];
                            let w = weight.data[o * taps + (ky * kernel + kx) * shape.channels + c];
                            sum += i64::from(value) * i64::from(w);
                        }
                    }
                }
                result[out.offset(ox, oy) + o] =
                    weight.bias[o] + sum as f32 * (in_scale * weight.scales[o]);
            }
        }
    }
    (out, result)
}

/// The plain depthwise convolution, weights `[tap][channel]`.
fn reference_depthwise(
    input: &[i16],
    in_scale: f32,
    shape: Shape,
    weight: &QWeight<'_>,
    kernel: usize,
    stride: usize,
    padding: usize,
) -> (Shape, Vec<f32>) {
    let channels = shape.channels;
    let out = Shape::new(
        output_size(shape.height, kernel, padding, stride),
        output_size(shape.width, kernel, padding, stride),
        channels,
    );
    let mut result = vec![0.0f32; out.len()];
    for oy in 0..out.height {
        for ox in 0..out.width {
            for c in 0..channels {
                let mut sum = 0i64;
                for ky in 0..kernel {
                    for kx in 0..kernel {
                        let iy = (oy * stride + ky) as isize - padding as isize;
                        let ix = (ox * stride + kx) as isize - padding as isize;
                        if iy < 0
                            || ix < 0
                            || iy >= shape.height as isize
                            || ix >= shape.width as isize
                        {
                            continue;
                        }
                        let value = input[shape.offset(ix as usize, iy as usize) + c];
                        let w = weight.data[(ky * kernel + kx) * channels + c];
                        sum += i64::from(value) * i64::from(w);
                    }
                }
                result[out.offset(ox, oy) + c] =
                    weight.bias[c] + sum as f32 * (in_scale * weight.scales[c]);
            }
        }
    }
    (out, result)
}

/// Random weights for `outputs` filters of `taps` values each.
fn random_weight(
    random: &mut Random,
    outputs: usize,
    taps: usize,
) -> (Vec<i8>, Vec<f32>, Vec<f32>) {
    let data = (0..outputs * taps).map(|_| random.i8()).collect();
    let scales = (0..outputs).map(|_| random.unit() * 0.01 + 1e-4).collect();
    let bias = (0..outputs).map(|_| random.unit() * 2.0 - 1.0).collect();
    (data, scales, bias)
}

/// Every value of `actual` equals `expected` to the bit.
fn assert_same_bits(what: &str, actual: &[f32], expected: &[f32]) {
    assert_eq!(actual.len(), expected.len(), "{what}: length");
    for (i, (a, e)) in actual.iter().zip(expected).enumerate() {
        assert_eq!(a.to_bits(), e.to_bits(), "{what}: value {i}: {a} vs {e}");
    }
}

/// Every value of `actual` is within one step of `expected` quantized.
fn assert_within_a_step(what: &str, actual: &[i16], expected: &[f32], quant: Quant) {
    for (i, (&a, &e)) in actual.iter().zip(expected).enumerate() {
        let expected = quant.quantize16(e);
        assert!(
            (i32::from(a) - i32::from(expected)).abs() <= 1,
            "{what}: value {i}: {a} vs {expected}"
        );
    }
}

#[test]
fn full_convolutions_match_the_plain_loop() {
    let mut random = Random(1);
    let quant = Quant {
        scale: 0.0003,
        zero_point: 0,
    };
    let out_quant = Quant {
        scale: 0.02,
        zero_point: 0,
    };
    for case in 0..60 {
        let kernel = 1 + random.below(4);
        let stride = 1 + random.below(2);
        let padding = random.below(kernel);
        let shape = Shape::new(
            kernel + random.below(6),
            kernel + random.below(6),
            1 + random.below(9),
        );
        // Every other case has a multiple of eight filters, packed.
        let packed = case % 2 == 1;
        let out_channels = if packed {
            8 * (1 + random.below(3))
        } else {
            1 + random.below(11)
        };
        let taps = kernel * kernel * shape.channels;
        if taps > 516 {
            continue;
        }
        let (data, scales, bias) = random_weight(&mut random, out_channels, taps);
        let plain = QWeight {
            data: &data,
            scales: &scales,
            bias: &bias,
            packed: false,
        };
        let input: Vec<i16> = (0..shape.len()).map(|_| random.i16()).collect();
        let (expected_shape, expected) =
            reference_conv2d(&input, quant.scale, shape, &plain, kernel, stride, padding);

        let mut packed_data = vec![0u8; data.len()];
        if packed {
            let bytes: Vec<u8> = data.iter().map(|&w| w as u8).collect();
            pack_rows(&bytes, out_channels, taps, &mut packed_data);
            for o in 0..out_channels {
                for k in 0..taps {
                    assert_eq!(
                        packed_data[packed_index(o, k, taps)] as i8,
                        data[o * taps + k]
                    );
                }
            }
        }
        let packed_data: Vec<i8> = packed_data.iter().map(|&w| w as i8).collect();
        let weight = if packed {
            QWeight {
                data: &packed_data,
                scales: &scales,
                bias: &bias,
                packed: true,
            }
        } else {
            plain
        };

        let mut actual = vec![0.0f32; expected.len()];
        let out = quant::conv2d_16(
            &input,
            quant,
            shape,
            &weight,
            kernel,
            stride,
            padding,
            Activation::None,
            Output::F32(&mut actual),
        );
        assert_eq!(out, expected_shape, "case {case}");
        assert_same_bits(&format!("conv2d_16 case {case}"), &actual, &expected);

        let mut actual = vec![0i16; expected.len()];
        quant::conv2d_16(
            &input,
            quant,
            shape,
            &weight,
            kernel,
            stride,
            padding,
            Activation::Relu,
            Output::I16(&mut actual, out_quant),
        );
        let relu: Vec<f32> = expected.iter().map(|v| v.max(0.0)).collect();
        assert_within_a_step(
            &format!("conv2d_16 i16 case {case}"),
            &actual,
            &relu,
            out_quant,
        );
    }
}

#[test]
fn depthwise_convolutions_match_the_plain_loop() {
    let mut random = Random(2);
    let quant = Quant {
        scale: 0.0003,
        zero_point: 0,
    };
    let out_quant = Quant {
        scale: 0.002,
        zero_point: 0,
    };
    for case in 0..60 {
        let kernel = 1 + random.below(9);
        let stride = 1 + random.below(2);
        let padding = random.below(kernel);
        let channels = 1 + random.below(13);
        let shape = Shape::new(kernel + random.below(5), kernel + random.below(5), channels);
        let (data, scales, bias) = random_weight(&mut random, channels, kernel * kernel);
        // The weight is [tap][channel]: regroup the random filters.
        let mut regrouped = vec![0i8; data.len()];
        for c in 0..channels {
            for tap in 0..kernel * kernel {
                regrouped[tap * channels + c] = data[c * kernel * kernel + tap];
            }
        }
        let weight = QWeight {
            data: &regrouped,
            scales: &scales,
            bias: &bias,
            packed: false,
        };
        let input: Vec<i16> = (0..shape.len()).map(|_| random.i16()).collect();
        let (expected_shape, expected) =
            reference_depthwise(&input, quant.scale, shape, &weight, kernel, stride, padding);
        let mut actual = vec![0.0f32; expected.len()];
        let out = quant::depthwise_16(
            &input,
            quant,
            shape,
            &weight,
            kernel,
            stride,
            padding,
            Activation::None,
            Output::F32(&mut actual),
        );
        assert_eq!(out, expected_shape, "case {case}");
        assert_same_bits(&format!("depthwise_16 case {case}"), &actual, &expected);

        let mut actual = vec![0i16; expected.len()];
        quant::depthwise_16(
            &input,
            quant,
            shape,
            &weight,
            kernel,
            stride,
            padding,
            Activation::Relu,
            Output::I16(&mut actual, out_quant),
        );
        let relu: Vec<f32> = expected.iter().map(|v| v.max(0.0)).collect();
        assert_within_a_step(
            &format!("depthwise_16 i16 case {case}"),
            &actual,
            &relu,
            out_quant,
        );
    }
}

#[test]
fn requantization_matches_the_float_formula() {
    let mut random = Random(5);
    let mut off_by_one = 0usize;
    let cases = 20_000;
    for _ in 0..cases {
        let scale = f32::exp((random.unit() - 0.5) * 30.0) * 1e-4;
        let bias = (random.unit() - 0.5) * 20.0;
        let step = f32::exp((random.unit() - 0.5) * 10.0) * 1e-3;
        let sum = match random.below(4) {
            0 => i64::from(random.next() as i32),
            1 => i64::from(random.next() as i32 % 1000),
            2 => (1i64 << 39) - i64::from(random.next() % 1000),
            _ => -(1i64 << 39) + i64::from(random.next() % 1000),
        };
        let requant = Requant::new(scale, bias, step);
        let exact = (f64::from(bias) + sum as f64 * f64::from(scale)) / f64::from(step);
        if exact.abs() >= f64::from(i32::MAX) {
            // Beyond the documented range: the result wraps.
            continue;
        }
        let actual = requant.apply(sum);
        let expected = exact.round() as i32;
        let difference = (i64::from(actual) - i64::from(expected)).abs();
        // The multiplier holds 31 bits, and a sum outside `i32` takes
        // the `f32` path: a relative error of 2^-23 at worst, which only
        // shows on outputs in the millions, plus the rounding.
        let allowed = 1 + (exact.abs() * 2f64.powi(-23)).ceil() as i64;
        assert!(
            difference <= allowed,
            "scale {scale}, bias {bias}, step {step}, sum {sum}: {actual} vs {expected} (exact {exact})"
        );
        if difference == 1 && allowed == 1 {
            // Only near a half-step.
            let fraction = (exact - exact.trunc()).abs();
            assert!(
                (fraction - 0.5).abs() < 1e-3,
                "scale {scale}, bias {bias}, step {step}, sum {sum}: {actual} vs {expected} (exact {exact})"
            );
            off_by_one += 1;
        }
    }
    assert!(
        off_by_one < cases / 100,
        "{off_by_one} results one step off"
    );
}

#[test]
fn accumulator_lanes_decode_as_signed_40_bit_values() {
    let mut bytes = [0u8; 64];
    // Lanes 0 to 3 in bytes 0..20, lanes 4 to 7 in bytes 32..52. Lane 0 =
    // 1, lane 1 = -1, lane 3 = 2^39 - 1, lane 4 = -2^39, lane 7 =
    // 0x12_3456_789A; the unused bytes hold noise.
    bytes[0] = 1;
    bytes[5..10].copy_from_slice(&[0xFF; 5]);
    bytes[15..20].copy_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF, 0x7F]);
    bytes[20..32].copy_from_slice(&[0xA5; 12]);
    bytes[32..37].copy_from_slice(&[0, 0, 0, 0, 0x80]);
    bytes[47..52].copy_from_slice(&[0x9A, 0x78, 0x56, 0x34, 0x12]);
    bytes[52..64].copy_from_slice(&[0x5A; 12]);
    let mut words = [0u32; 16];
    for (word, chunk) in words.iter_mut().zip(bytes.chunks_exact(4)) {
        *word = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
    }
    let lanes = decode_lanes(&words);
    assert_eq!(lanes[0], 1);
    assert_eq!(lanes[1], -1);
    assert_eq!(lanes[2], 0);
    assert_eq!(lanes[3], (1i64 << 39) - 1);
    assert_eq!(lanes[4], -(1i64 << 39));
    assert_eq!(lanes[7], 0x12_3456_789A);
}

#[test]
fn inline_rounding_matches_libm_on_ties_and_near_ties() {
    let mut random = Random(4);
    let mut cases: Vec<f32> = vec![
        0.0,
        -0.0,
        0.5,
        -0.5,
        1.5,
        -1.5,
        2.5,
        -2.5,
        0.49999997,
        -0.49999997,
    ];
    for _ in 0..100_000 {
        let magnitude = match random.below(4) {
            0 => 1.0,
            1 => 100.0,
            2 => 40_000.0,
            _ => 8_388_608.0,
        };
        let x = (random.unit() * 2.0 - 1.0) * magnitude;
        cases.push(x);
        // Exactly on and next to a half.
        let half = x.trunc() + 0.5f32.copysign(x);
        cases.push(half);
        cases.push(f32::from_bits(half.to_bits() + 1));
        cases.push(f32::from_bits(half.to_bits() - 1));
    }
    for &x in &cases {
        assert_eq!(
            round_to_int(x),
            libm::roundf(x) as i32,
            "x = {x} ({:#x})",
            x.to_bits()
        );
    }
    assert_eq!(round_to_int(f32::MAX), i32::MAX);
    assert_eq!(round_to_int(f32::MIN), i32::MIN);
    assert_eq!(round_to_int(f32::NAN), 0);
}
