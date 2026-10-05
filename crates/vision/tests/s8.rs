//! The 8-bit kernels on the computer: the accumulator image's packing,
//! the kernels against plain references in `f64`, and the banded block
//! against the whole one.

use hack_and_hike_vision::nn::s8::{
    self, Depthwise, GroupPrelu, LANES, Plan, Pointwise, Prelu, Store,
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
    let alpha: [i8; LANES] = core::array::from_fn(|i| i as i8 - 8);
    let prelu = GroupPrelu {
        alpha,
        positive: 1,
        shift: 7,
    };
    let plan = Plan::new(&bias, 7, Some(&prelu));
    assert_eq!(decode_lanes(&plan.image), bias.map(|b| b + 64));
    assert_eq!(decode_lanes(&plan.prelu_image), [64; LANES]);
    assert_eq!(plan.alpha, alpha);
    assert_eq!(
        (plan.positive, plan.prelu_shift, plan.shift, plan.prelu),
        (1, 7, 7, 1)
    );
    assert_eq!(plan.group_prelu(), Some(prelu));
    let plain = Plan::new(&bias, 0, None);
    assert_eq!((plain.image, plain.prelu), (encode_lanes(&bias), 0));
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

/// A layer's random weights and biases, in the kernels' layout and plain,
/// and its plans; every group with the same shifts until [`Weights::reshift`].
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

    /// Give group `group` its own shift (and PReLU shift), as ESP-DL's
    /// split layers have.
    fn reshift(&mut self, group: usize, shift: u32) {
        let mut bias = [0i32; LANES];
        bias.copy_from_slice(&self.bias[group * LANES..(group + 1) * LANES]);
        let prelu = self.plans[group].group_prelu().map(|p| GroupPrelu {
            shift: p.shift + 1,
            ..p
        });
        self.plans[group] = Plan::new(&bias, shift, prelu.as_ref());
    }

    /// The expected value of channel `o` for the products `sum`.
    fn expected(&self, o: usize, sum: f64) -> i8 {
        let plan = &self.plans[o / LANES];
        let prelu = plan
            .group_prelu()
            .map(|p| (&self.alpha[..], p.positive, p.shift));
        reference_value(sum, plan.shift, prelu, o)
    }

    fn pointwise(&self, input: usize) -> Pointwise<'_> {
        Pointwise {
            input,
            weights: &self.packed,
            plans: &self.plans,
        }
    }

    fn depthwise(&self, stride: usize) -> Depthwise<'_> {
        Depthwise {
            weights: &self.packed,
            plans: &self.plans,
            stride,
        }
    }
}

#[test]
fn pointwise_matches_the_plain_reference() {
    let mut random = Random(7);
    let cases = [
        (5, 16, 16, 6, None, Store::Write, false),
        (9, 64, 128, 8, Some((1, 7)), Store::Write, true),
        (3, 512, 64, 9, Some((0, 6)), Store::Add, false),
        (1, 32, 48, 0, None, Store::Add, true),
    ];
    for (pixels, input, output, shift, prelu, store, split) in cases {
        let mut w = Weights::random(&mut random, input, output, shift, prelu);
        if split {
            w.reshift(output / LANES - 1, shift + 1);
        }
        let layer = w.pointwise(input);
        let data = random.bytes(pixels * input, 64);
        let before = random.bytes(pixels * output, 128);
        let mut out = before.clone();
        s8::pointwise(&layer, store, &data, &mut out);
        for (p, pixel) in data.chunks_exact(input).enumerate() {
            for o in 0..output {
                let sum = f64::from(w.bias[o])
                    + pixel
                        .iter()
                        .zip(&w.plain[o * input..(o + 1) * input])
                        .map(|(&x, &w)| f64::from(x) * f64::from(w))
                        .sum::<f64>();
                let mut expected = w.expected(o, sum);
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
        let w = Weights::random(&mut random, 9, channels, 6, Some((1, 7)));
        let layer = w.depthwise(stride);
        let data = random.bytes(height * width * channels, 64);
        let row = width * channels;
        let out_width = layer.output_size(width);
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
                    assert_eq!(
                        out[ox * channels + c],
                        w.expected(c, sum),
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
    let shapes = [
        (6, 5, 16, 32, 16, 1, true),
        (14, 14, 128, 256, 128, 1, true),
        (1, 3, 32, 64, 32, 1, true),
        (8, 8, 32, 64, 48, 2, false),
        (7, 9, 16, 32, 16, 2, false),
        (5, 4, 16, 32, 32, 1, false),
    ];
    for (height, width, channels, expanded, outputs, stride, residual) in shapes {
        let expand = Weights::random(&mut random, channels, expanded, 8, Some((0, 7)));
        let depth = Weights::random(&mut random, 9, expanded, 6, Some((1, 7)));
        let project = Weights::random(&mut random, expanded, outputs, 9, None);
        let block = Block {
            height,
            width,
            expand: expand.pointwise(channels),
            depthwise: depth.depthwise(stride),
            project: project.pointwise(expanded),
            residual,
        };
        let input = random.bytes(height * width * channels, 64);
        let mut whole = vec![0i8; block.output_len()];
        let mut wide = vec![0i8; block.wide_len()];
        let mut filtered = vec![0i8; block.filtered_whole_len()];
        block::run_whole(&block, &input, &mut wide, &mut filtered, &mut whole);
        for band in [1, 2, 3, 4, 7, 20] {
            let mut banded = vec![0i8; block.output_len()];
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
            assert_eq!(
                banded, whole,
                "{height}x{width}x{channels}/{stride}, band {band}"
            );
        }
        assert!(
            whole.iter().any(|&v| v != 0),
            "the block computed something"
        );
        assert_eq!(
            block.band_for(
                block.ring_len(3),
                block.filtered_len(3),
                block.staging_len(3)
            ),
            3.min(block.out_height())
        );
    }
}

#[test]
fn whole_block_matches_the_layers_one_by_one() {
    let mut random = Random(17);
    let (height, width, channels, expanded) = (4, 6, 16, 32);
    let expand = Weights::random(&mut random, channels, expanded, 8, Some((0, 7)));
    let depth = Weights::random(&mut random, 9, expanded, 6, Some((1, 7)));
    let project = Weights::random(&mut random, expanded, channels, 9, None);
    let block = Block {
        height,
        width,
        expand: expand.pointwise(channels),
        depthwise: depth.depthwise(1),
        project: project.pointwise(expanded),
        residual: true,
    };
    let input = random.bytes(height * width * channels, 64);
    let mut out = vec![0i8; block.output_len()];
    let mut wide = vec![0i8; block.wide_len()];
    let mut filtered = vec![0i8; block.filtered_whole_len()];
    block::run_whole(&block, &input, &mut wide, &mut filtered, &mut out);
    let mut wide_ref = vec![0i8; block.wide_len()];
    model::pointwise(&block.expand, Store::Write, &input, &mut wide_ref);
    assert_eq!(wide, wide_ref);
    let mut expected = input.clone();
    model::pointwise(&block.project, Store::Add, &filtered, &mut expected);
    assert_eq!(out, expected);
}
