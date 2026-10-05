//! The 8-bit kernels on the computer: the accumulator image's packing,
//! the kernels against plain references in `f64`, and the banded block
//! against the whole one.

use hack_and_hike_vision::nn::s8::{
    self, Depthwise, LANES, Plan, Pointwise, Prelu, Store,
    block::{self, Block},
    decode_lanes, encode_lanes, model,
};

/// A small deterministic generator.
struct Random(u64);

impl Random {
    fn next(&mut self) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) as u32
    }

    /// A value in `-bound..bound`.
    fn signed(&mut self, bound: i32) -> i32 {
        (self.next() % (2 * bound as u32)) as i32 - bound
    }

    fn bytes(&mut self, len: usize, bound: i32) -> Vec<i8> {
        (0..len).map(|_| self.signed(bound) as i8).collect()
    }
}

#[test]
fn lanes_round_trip_through_the_image() {
    let mut random = Random(1);
    for _ in 0..1000 {
        let values: [i32; LANES] = core::array::from_fn(|_| random.signed(1 << 19));
        assert_eq!(decode_lanes(&encode_lanes(&values)), values);
    }
    let extremes = [
        -(1 << 19),
        (1 << 19) - 1,
        -1,
        0,
        1,
        2,
        -2,
        3,
        -3,
        4,
        -4,
        5,
        -5,
        6,
        -6,
        7,
    ];
    assert_eq!(decode_lanes(&encode_lanes(&extremes)), extremes);
}

#[test]
fn image_puts_lane_i_at_bit_20_i() {
    let mut values = [0i32; LANES];
    values[1] = 1;
    values[9] = -1;
    let words = encode_lanes(&values);
    assert_eq!(words[0], 1 << 20, "lane 1 starts at bit 20 of the low half");
    assert_eq!(
        words[8], 0xFFF0_0000,
        "lane 9: bits 20..32 of the high half"
    );
    assert_eq!(words[9], 0xFF, "lane 9: bits 32..40 of the high half");
    assert_eq!(words[5..8], [0; 3], "padding after the low half");
    assert_eq!(words[13..16], [0; 3], "padding after the high half");
}

#[test]
fn plan_holds_what_the_assembly_reads() {
    assert_eq!(core::mem::size_of::<Plan>(), s8::PLAN_BYTES);
    let bias: [i32; LANES] = core::array::from_fn(|i| i as i32 * 100 - 800);
    let alpha: Vec<i8> = (0..32).map(|i| i as i8 - 16).collect();
    let prelu = Prelu {
        alpha: &alpha,
        positive: 1,
        shift: 7,
    };
    let plan = Plan::new(&bias, 7, Some((&prelu, 16)));
    assert_eq!(decode_lanes(&plan.image), bias.map(|b| b + 64));
    assert_eq!(decode_lanes(&plan.prelu_image), [64; LANES]);
    assert_eq!(plan.alpha[..], alpha[16..32]);
    assert_eq!((plan.positive, plan.prelu_shift), (1, 7));
    assert_eq!(Plan::new(&bias, 0, None).image, encode_lanes(&bias));
}

#[test]
fn requantize_rounds_half_up_and_saturates() {
    assert_eq!(model::requantize(3, 1), 2);
    assert_eq!(model::requantize(-3, 1), -1);
    assert_eq!(model::requantize(-5, 1), -2);
    assert_eq!(model::requantize(1000, 2), 127);
    assert_eq!(model::requantize(-1000, 2), -128);
    assert_eq!(model::requantize(-7, 0), -7);
}

/// The plain reference of a requantization and a PReLU, in `f64`.
fn reference_value(sum: f64, shift: u32, prelu: Option<(&[i8], u32, u32)>, channel: usize) -> i8 {
    let round = |v: f64, shift: u32| {
        (v / f64::from(1u32 << shift) + 0.5)
            .floor()
            .clamp(-128.0, 127.0)
    };
    let value = round(sum, shift);
    let Some((alpha, positive, prelu_shift)) = prelu else {
        return value as i8;
    };
    if value >= 0.0 {
        (value * f64::from(1u32 << positive)).min(127.0) as i8
    } else {
        round(value * f64::from(alpha[channel]), prelu_shift) as i8
    }
}

/// A layer's random weights and biases, in the kernels' layout and plain.
struct Weights {
    /// `[output / 16][taps][16]`.
    packed: Vec<i8>,
    /// `w[o][tap]`.
    plain: Vec<i8>,
    bias: Vec<i32>,
    alpha: Vec<i8>,
    plans: Vec<Plan>,
}

impl Weights {
    fn random(
        random: &mut Random,
        taps: usize,
        output: usize,
        shift: u32,
        prelu: Option<(u32, u32)>,
    ) -> Self {
        let plain = random.bytes(taps * output, 64);
        let mut packed = vec![0i8; taps * output];
        for o in 0..output {
            for t in 0..taps {
                packed[(o / LANES * taps + t) * LANES + o % LANES] = plain[o * taps + t];
            }
        }
        let bias: Vec<i32> = (0..output).map(|_| random.signed(4096)).collect();
        let alpha = random.bytes(output, 120);
        let mut plans = vec![Plan::ZERO; output / LANES];
        let activation = prelu.map(|(positive, shift)| Prelu {
            alpha: &alpha,
            positive,
            shift,
        });
        s8::plans(&bias, shift, activation.as_ref(), &mut plans);
        Self {
            packed,
            plain,
            bias,
            alpha,
            plans,
        }
    }

    fn prelu(&self, prelu: Option<(u32, u32)>) -> Option<Prelu<'_>> {
        prelu.map(|(positive, shift)| Prelu {
            alpha: &self.alpha,
            positive,
            shift,
        })
    }
}

#[test]
fn pointwise_matches_the_plain_reference() {
    let mut random = Random(7);
    let cases = [
        (5, 16, 16, 6, None, Store::Write),
        (9, 64, 128, 8, Some((1, 7)), Store::Write),
        (3, 512, 64, 9, Some((0, 6)), Store::Add),
        (1, 32, 48, 0, None, Store::Add),
    ];
    for (pixels, input, output, shift, prelu, store) in cases {
        let w = Weights::random(&mut random, input, output, shift, prelu);
        let layer = Pointwise {
            input,
            weights: &w.packed,
            bias: &w.bias,
            plans: &w.plans,
            shift,
            prelu: w.prelu(prelu),
        };
        let data = random.bytes(pixels * input, 64);
        let before = random.bytes(pixels * output, 128);
        let mut out = before.clone();
        s8::pointwise(&layer, store, &data, &mut out);
        let alpha = prelu.map(|(positive, shift)| (&w.alpha[..], positive, shift));
        for (p, pixel) in data.chunks_exact(input).enumerate() {
            for o in 0..output {
                let sum = f64::from(w.bias[o])
                    + pixel
                        .iter()
                        .zip(&w.plain[o * input..(o + 1) * input])
                        .map(|(&x, &w)| f64::from(x) * f64::from(w))
                        .sum::<f64>();
                let mut expected = reference_value(sum, shift, alpha, o);
                if store == Store::Add {
                    expected = before[p * output + o].saturating_add(expected);
                }
                assert_eq!(
                    out[p * output + o],
                    expected,
                    "pixel {p} channel {o} of {pixels}x{input}->{output}"
                );
            }
        }
    }
}

#[test]
fn depthwise_matches_the_plain_reference() {
    let mut random = Random(11);
    for (height, width, channels, stride) in [(4, 5, 32, 1), (5, 7, 16, 2), (1, 1, 16, 1)] {
        let prelu = Some((1, 7));
        let w = Weights::random(&mut random, 9, channels, 6, prelu);
        let layer = Depthwise {
            channels,
            weights: &w.packed,
            bias: &w.bias,
            plans: &w.plans,
            shift: 6,
            stride,
            prelu: w.prelu(prelu),
        };
        let data = random.bytes(height * width * channels, 64);
        let row = width * channels;
        let out_width = layer.output_width(width);
        for oy in (0..height).step_by(stride) {
            let rows = [
                oy.checked_sub(1).map(|r| &data[r * row..(r + 1) * row]),
                Some(&data[oy * row..(oy + 1) * row]),
                (oy + 1 < height).then(|| &data[(oy + 1) * row..(oy + 2) * row]),
            ];
            let mut out = vec![0i8; out_width * channels];
            s8::depthwise_row(&layer, rows, width, &mut out);
            for ox in 0..out_width {
                for c in 0..channels {
                    let mut sum = f64::from(w.bias[c]);
                    for ky in 0..3 {
                        for kx in 0..3 {
                            let (y, x) =
                                ((oy + ky).checked_sub(1), (ox * stride + kx).checked_sub(1));
                            let (Some(y), Some(x)) = (y, x) else { continue };
                            if y >= height || x >= width {
                                continue;
                            }
                            sum += f64::from(data[(y * width + x) * channels + c])
                                * f64::from(w.plain[c * 9 + ky * 3 + kx]);
                        }
                    }
                    let expected = reference_value(sum, 6, Some((&w.alpha, 1, 7)), c);
                    assert_eq!(
                        out[ox * channels + c],
                        expected,
                        "row {oy} pixel {ox} channel {c}"
                    );
                }
            }
        }
    }
}

#[test]
fn banded_block_equals_whole_block() {
    let mut random = Random(13);
    for (height, width, channels) in [(6, 5, 16), (14, 14, 128), (1, 3, 32)] {
        let expanded = 2 * channels;
        let expand = Weights::random(&mut random, channels, expanded, 8, Some((0, 7)));
        let depth = Weights::random(&mut random, 9, expanded, 6, Some((1, 7)));
        let project = Weights::random(&mut random, expanded, channels, 9, None);
        let block = Block {
            height,
            width,
            expand: Pointwise {
                input: channels,
                weights: &expand.packed,
                bias: &expand.bias,
                plans: &expand.plans,
                shift: 8,
                prelu: expand.prelu(Some((0, 7))),
            },
            depthwise: Depthwise {
                channels: expanded,
                weights: &depth.packed,
                bias: &depth.bias,
                plans: &depth.plans,
                shift: 6,
                stride: 1,
                prelu: depth.prelu(Some((1, 7))),
            },
            project: Pointwise {
                input: expanded,
                weights: &project.packed,
                bias: &project.bias,
                plans: &project.plans,
                shift: 9,
                prelu: None,
            },
        };
        let input = random.bytes(height * width * channels, 64);
        let mut whole = vec![0i8; input.len()];
        let (mut wide, mut filtered) = (vec![0i8; block.whole_len()], vec![0i8; block.whole_len()]);
        block::run_whole(&block, &input, &mut wide, &mut filtered, &mut whole);
        for band in [1, 2, 3, 4, 7, 20] {
            let mut banded = vec![0i8; input.len()];
            let mut ring = vec![0i8; block.ring_len(band)];
            let mut filtered = vec![0i8; block.filtered_len(band)];
            let mut staging = vec![0i8; block.staging_len(band)];
            block::run_banded(
                &block,
                band,
                &input,
                &mut ring,
                &mut filtered,
                &mut staging,
                &mut banded,
            );
            assert_eq!(banded, whole, "{height}x{width}x{channels}, band {band}");
        }
        assert_ne!(whole, input, "the block changed something");
    }
}
