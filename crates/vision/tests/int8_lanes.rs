//! The lane kernels against the `f32` reference: each one alone on random
//! data, so a wrong plan or store shows where it is.

use hack_and_hike_vision::nn::{
    Activation, Shape, conv2d, depthwise,
    lanes::{
        self, GELU_STEP, GeluTable, GroupPlan, LANES, LaneWeight, NormPlan, Store, model,
        plan_channels, plan_groups,
    },
    linear, standardize_rows,
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

    fn below(&mut self, bound: usize) -> usize {
        self.next() as usize % bound
    }

    fn unit(&mut self) -> f32 {
        self.next() as f32 / u32::MAX as f32
    }

    fn signed(&mut self) -> f32 {
        self.unit() * 2.0 - 1.0
    }
}

/// Quantize `values` symmetrically to `i16` with `step`.
fn quantize(values: &[f32], step: f32) -> Vec<i16> {
    values
        .iter()
        .map(|&v| (v / step).round().clamp(-32768.0, 32767.0) as i16)
        .collect()
}

/// The `i8` weights and per-output scales of `rows` of `per_output`.
fn quantize_rows(rows: &[f32], per_output: usize) -> (Vec<i8>, Vec<f32>) {
    let mut data = vec![0i8; rows.len()];
    let mut scales = vec![0.0f32; rows.len() / per_output];
    hack_and_hike_vision::nn::quant::quantize_weight_rows(rows, per_output, &mut data, &mut scales);
    (data, scales)
}

/// The signal-to-noise ratio of `actual` (i16, `step`) against `expected`.
fn snr(expected: &[f32], actual: &[i16], step: f32) -> f32 {
    let back: Vec<f32> = actual.iter().map(|&q| f32::from(q) * step).collect();
    hack_and_hike_vision::nn::quant::snr_db(expected, &back)
}

#[test]
fn the_group_epilogue_matches_the_formula() {
    let mut random = Random(1);
    for case in 0..2000 {
        let mut scale = [0.0f32; LANES];
        let mut bias = [0.0f32; LANES];
        for j in 0..LANES {
            // Scales within a factor of sixteen of each other in a group,
            // as in the models; one in eight negative (a folded gamma).
            scale[j] = 10f32.powf(random.unit() * 1.2 - 4.0)
                * if random.below(8) == 0 { -1.0 } else { 1.0 };
            bias[j] = random.signed() * 3.0;
        }
        let step = 10f32.powf(random.signed() * 2.0 - 3.0);
        let products = 8 + random.below(600);
        let plan = GroupPlan::new(&scale, &bias, step, lanes::sum_bound(products));
        let mut sums = lanes::decode_lanes(&plan.image);
        let mut expected = [0i16; LANES];
        for j in 0..LANES {
            // A sum within the output's range.
            let target = random.signed() * 32767.0 * step;
            let sum = ((target - bias[j]) / scale[j]).round();
            let sum = sum.clamp(
                -(lanes::sum_bound(products) as f32),
                lanes::sum_bound(products) as f32,
            );
            sums[j] += sum as i64;
            let real = bias[j] + sum * scale[j];
            expected[j] = (real / step).round().clamp(-32768.0, 32767.0) as i16;
        }
        let actual = model::epilogue(&sums, &plan);
        for j in 0..LANES {
            let difference = (i32::from(actual[j]) - i32::from(expected[j])).abs();
            // The first shift is shared by the group: a lane whose scale
            // is larger than its neighbours' keeps fewer bits, so its
            // rounding is worth up to half a shifted unit in output
            // steps; plus the factor's rounding and the second shift.
            let unit =
                (i32::from(plan.factor[j]).abs() as f64 / f64::from(1u32 << plan.s2)).ceil() as i32;
            let allowed = 2 + unit / 2 + (i32::from(expected[j]).abs() >> 12);
            assert!(
                difference <= allowed,
                "case {case} lane {j}: {} vs {} (scale {}, bias {}, step {step}, plan {plan:?})",
                actual[j],
                expected[j],
                scale[j],
                bias[j]
            );
        }
    }
}

#[test]
fn the_lane_linear_matches_the_float_one() {
    let mut random = Random(2);
    for case in 0..20 {
        let in_features = 2 * (1 + random.below(40));
        let outputs = 8 * (1 + random.below(6));
        let rows = 1 + random.below(5);
        let input: Vec<f32> = (0..rows * in_features)
            .map(|_| random.signed() * 3.0)
            .collect();
        let weight: Vec<f32> = (0..outputs * in_features)
            .map(|_| random.signed() * 0.5)
            .collect();
        let bias: Vec<f32> = (0..outputs).map(|_| random.signed()).collect();
        let mut expected = vec![0.0f32; rows * outputs];
        linear(
            &input,
            in_features,
            &weight,
            Some(&bias),
            outputs,
            Activation::None,
            &mut expected,
        );
        let in_step = 3.0 / 32767.0;
        let input_q = quantize(&input, in_step);
        let (data, scales) = quantize_rows(&weight, in_features);
        let mut packed = vec![0i8; data.len()];
        lanes::pack_weight(&data, outputs, in_features, &mut packed);
        let largest = expected.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        let out_step = largest / 32767.0;
        let mut plans = vec![GroupPlan::ZERO; outputs / LANES];
        plan_groups(&scales, &bias, in_step, out_step, in_features, &mut plans);
        let weight = LaneWeight {
            data: &packed,
            per_output: in_features,
            plans: &plans,
        };
        let mut actual = vec![0i16; rows * outputs];
        lanes::linear(&input_q, in_features, &weight, Store::Write, &mut actual);
        let snr = snr(&expected, &actual, out_step);
        println!("linear case {case}: {snr:.1} dB");
        assert!(snr > 40.0, "case {case}: {snr} dB");

        // The residual store adds to what is there.
        let stream: Vec<i16> = (0..rows * outputs)
            .map(|_| random.below(20000) as i16 - 10000)
            .collect();
        let mut added = stream.clone();
        lanes::linear(&input_q, in_features, &weight, Store::Add, &mut added);
        for i in 0..added.len() {
            assert_eq!(
                added[i],
                stream[i].saturating_add(actual[i]),
                "add case {case} value {i}"
            );
        }
        // Relu.
        let mut relu = vec![0i16; rows * outputs];
        lanes::linear(&input_q, in_features, &weight, Store::Relu, &mut relu);
        for i in 0..relu.len() {
            assert_eq!(relu[i], actual[i].max(0), "relu case {case} value {i}");
        }
    }
}

#[test]
fn the_lane_convolution_matches_the_float_one() {
    let mut random = Random(3);
    for case in 0..20 {
        let kernel = 1 + random.below(4);
        let stride = 1 + random.below(2);
        let padding = random.below(kernel);
        let channels = 2 * (1 + random.below(6));
        let shape = Shape::new(kernel + random.below(5), kernel + random.below(5), channels);
        let outputs = 8 * (1 + random.below(3));
        let taps = kernel * kernel * channels;
        let input: Vec<f32> = (0..shape.len()).map(|_| random.signed() * 2.0).collect();
        let weight: Vec<f32> = (0..outputs * taps).map(|_| random.signed() * 0.5).collect();
        let bias: Vec<f32> = (0..outputs).map(|_| random.signed()).collect();
        let out = Shape::new(
            (shape.height + 2 * padding - kernel) / stride + 1,
            (shape.width + 2 * padding - kernel) / stride + 1,
            outputs,
        );
        let mut expected = vec![0.0f32; out.len()];
        conv2d(
            &input,
            shape,
            &weight,
            &bias,
            kernel,
            stride,
            padding,
            Activation::None,
            &mut expected,
        );
        let in_step = 2.0 / 32767.0;
        let input_q = quantize(&input, in_step);
        let (data, scales) = quantize_rows(&weight, taps);
        let mut packed = vec![0i8; data.len()];
        lanes::pack_weight(&data, outputs, taps, &mut packed);
        let largest = expected.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        let out_step = largest / 32767.0;
        let mut plans = vec![GroupPlan::ZERO; outputs / LANES];
        plan_groups(&scales, &bias, in_step, out_step, taps, &mut plans);
        let weight = LaneWeight {
            data: &packed,
            per_output: taps,
            plans: &plans,
        };
        let mut actual = vec![0i16; out.len()];
        let produced = lanes::conv2d(
            &input_q,
            shape,
            &weight,
            kernel,
            stride,
            padding,
            Store::Write,
            &mut actual,
        );
        assert_eq!(produced, out);
        let snr = snr(&expected, &actual, out_step);
        println!("conv2d case {case}: {snr:.1} dB");
        assert!(snr > 40.0, "case {case}: {snr} dB");
    }
}

#[test]
fn the_lane_depthwise_matches_the_float_one() {
    let mut random = Random(4);
    for case in 0..20 {
        let kernel = 1 + random.below(5);
        let padding = random.below(kernel);
        let channels = 8 * (1 + random.below(4));
        let shape = Shape::new(kernel + random.below(5), kernel + random.below(5), channels);
        let input: Vec<f32> = (0..shape.len()).map(|_| random.signed() * 2.0).collect();
        let weight: Vec<f32> = (0..kernel * kernel * channels)
            .map(|_| random.signed() * 0.5)
            .collect();
        let bias: Vec<f32> = (0..channels).map(|_| random.signed()).collect();
        let out = Shape::new(
            shape.height + 2 * padding - kernel + 1,
            shape.width + 2 * padding - kernel + 1,
            channels,
        );
        let mut expected = vec![0.0f32; out.len()];
        depthwise(
            &input,
            shape,
            &weight,
            &bias,
            kernel,
            1,
            padding,
            Activation::None,
            &mut expected,
        );
        let in_step = 2.0 / 32767.0;
        let input_q = quantize(&input, in_step);
        let mut data = vec![0i8; weight.len()];
        let mut scales = vec![0.0f32; channels];
        hack_and_hike_vision::nn::quant::quantize_weight_channels(
            &weight,
            channels,
            &mut data,
            &mut scales,
        );
        let largest = expected.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        let out_step = largest / 32767.0;
        let mut plans = vec![GroupPlan::ZERO; channels / LANES];
        plan_channels(
            &scales,
            &bias,
            &[in_step],
            out_step,
            kernel * kernel,
            &mut plans,
        );
        let mut actual = vec![0i16; out.len()];
        let produced = lanes::depthwise(
            &input_q,
            shape,
            &data,
            &plans,
            kernel,
            1,
            padding,
            Store::Write,
            &mut actual,
        );
        assert_eq!(produced, out);
        let snr = snr(&expected, &actual, out_step);
        println!("depthwise case {case}: {snr:.1} dB");
        assert!(snr > 40.0, "case {case}: {snr} dB");
    }
}

#[test]
fn the_lane_layer_norm_matches_the_float_one() {
    let mut random = Random(5);
    for case in 0..20 {
        let channels = 8 * (1 + random.below(20));
        let rows = 1 + random.below(10);
        let input: Vec<f32> = (0..rows * channels)
            .map(|_| random.signed() * 3.0 + 0.5)
            .collect();
        let gain: Vec<f32> = (0..channels).map(|_| random.signed() * 2.0).collect();
        let bias: Vec<f32> = (0..channels).map(|_| random.signed()).collect();
        let mut expected = input.clone();
        standardize_rows(&mut expected, channels, 1e-6);
        for row in expected.chunks_exact_mut(channels) {
            for (c, v) in row.iter_mut().enumerate() {
                *v = *v * gain[c] + bias[c];
            }
        }
        let in_step = 4.0 / 32767.0;
        let input_q = quantize(&input, in_step);
        let out_step = 8.0 / 32767.0;
        let plans: Vec<NormPlan> = (0..channels / LANES)
            .map(|g| {
                let mut gains = [0.0f32; LANES];
                let mut biases = [0.0f32; LANES];
                for j in 0..LANES {
                    gains[j] = gain[g * LANES + j];
                    biases[j] = bias[g * LANES + j];
                }
                NormPlan::new(&gains, &biases, &[out_step; LANES])
            })
            .collect();
        let mut actual = vec![0i16; rows * channels];
        lanes::layer_norm(&input_q, channels, in_step, 1e-6, &plans, &mut actual);
        let snr = snr(&expected, &actual, out_step);
        println!("layer_norm case {case}: {snr:.1} dB");
        assert!(snr > 40.0, "case {case}: {snr} dB");
    }
}

#[test]
fn the_gelu_table_matches_the_function() {
    let mut storage = vec![0i16; GeluTable::LEN];
    let table = GeluTable::build(&mut storage);
    let mut values: Vec<i16> = (-16384..16384).step_by(7).collect();
    let expected: Vec<f32> = values
        .iter()
        .map(|&x| {
            let mut y = [f32::from(x) * GELU_STEP];
            hack_and_hike_vision::nn::gelu(&mut y);
            y[0]
        })
        .collect();
    table.apply(&mut values);
    let snr = snr(&expected, &values, GELU_STEP);
    println!("gelu table: {snr:.1} dB");
    assert!(snr > 60.0, "{snr} dB");
}
