//! The arithmetic of the lane kernels in scalar code: exactly what the
//! vector unit computes, so the computer and the board agree bit for
//! bit. The application's self-test checks both networks built from
//! these on the board against the computer ([`check`](crate::nn::check));
//! `docs/face_id/performance.md` says how to check each assembly
//! primitive on its own again.

use super::{GroupPlan, LANES, LaneWeight, NormPlan, Run, Store, Tap, decode_lanes};

/// The GELU input's clamp, in units of `2^-10`.
const GELU_LIMIT: i16 = 16384;

/// Saturate to 16 bits.
#[inline(always)]
pub fn sat16(value: i64) -> i16 {
    value.clamp(-32768, 32767) as i16
}

/// The epilogue of one group: the eight lane sums (which already hold
/// the bias image) to eight output values.
///
/// `ee.srcmb.s16.qacc` by `s1`: an arithmetic shift (floor) saturated to
/// 16 bits; `ee.vadds.s16` of `half`; a multiply-accumulate by `factor`
/// from a zeroed accumulator; `ee.srcmb.s16.qacc` by `s2`;
/// `ee.vadds.s16` of `offset`.
#[inline]
pub fn epilogue(sums: &[i64; LANES], plan: &GroupPlan) -> [i16; LANES] {
    let mut out = [0i16; LANES];
    for j in 0..LANES {
        let shifted = sat16(sums[j] >> plan.s1);
        let rounded = sat16(i64::from(shifted) + i64::from(plan.half[j]));
        let product = i64::from(rounded) * i64::from(plan.factor[j]);
        let scaled = sat16(product >> plan.s2);
        out[j] = scaled.saturating_add(plan.offset[j]);
    }
    out
}

/// Store eight finished values as `store` says.
#[inline]
pub fn store(values: &[i16; LANES], store: Store<'_>, out: &mut [i16]) {
    match store {
        Store::Write => out[..LANES].copy_from_slice(values),
        Store::Add => {
            for (o, &v) in out.iter_mut().zip(values) {
                *o = o.saturating_add(v);
            }
        }
        Store::Relu => {
            for (o, &v) in out.iter_mut().zip(values) {
                *o = v.max(0);
            }
        }
        Store::Gelu(_) => {
            for (o, &v) in out.iter_mut().zip(values) {
                *o = v.clamp(-GELU_LIMIT, GELU_LIMIT);
            }
        }
    }
}

/// `sat(value * factor >> shift)`.
#[inline(always)]
pub fn scale16(value: i16, factor: i16, shift: u32) -> i16 {
    sat16((i64::from(value) * i64::from(factor)) >> shift)
}

/// The per-row constants of a LayerNorm: the mean of the row (in input
/// units), and the factor and shift that take `value - mean` to the
/// standardized value in units of `2^-11`: `1 / sqrt(variance +
/// epsilon)` with `step` folded in.
///
/// The definition, from the squares about the mean. The kernels compute
/// the same constants from the sums of a row ([`norm_constants`]); the
/// tests check that both agree.
pub fn norm_row(row: &[i16], step: f32, epsilon: f32) -> (i16, i16, u32) {
    let n = row.len() as i64;
    let sum: i64 = row.iter().map(|&v| i64::from(v)).sum();
    let mean = sat16((2 * sum + n).div_euclid(2 * n));
    let mut squares = 0i64;
    for &v in row {
        let d = i64::from(v) - i64::from(mean);
        squares += d * d;
    }
    norm_scale(n, mean, squares, step, epsilon)
}

/// The sum of a row's values and the sum of their squares, exact.
pub fn row_sums(row: &[i16]) -> (i64, i64) {
    row.iter().fold((0, 0), |(sum, squares), &v| {
        let v = i64::from(v);
        (sum + v, squares + v * v)
    })
}

/// [`norm_row`] from the sums of [`row_sums`] of a row of `len` values:
/// the same constants. The squares about the mean are `squares - 2 *
/// mean * sum + len * mean^2`, exact in integers, and the mean's division
/// runs in 32 bits (a 64-bit division is a software routine on the
/// board); a row's sum is far inside `i32`.
pub fn norm_constants(
    len: usize,
    sum: i64,
    squares: i64,
    step: f32,
    epsilon: f32,
) -> (i16, i16, u32) {
    let n = len as i64;
    let mean = match (i32::try_from(2 * sum + n), i32::try_from(2 * n)) {
        (Ok(twice), Ok(divisor)) => sat16(i64::from(twice.div_euclid(divisor))),
        _ => sat16((2 * sum + n).div_euclid(2 * n)),
    };
    let m = i64::from(mean);
    let centred = squares - 2 * m * sum + n * m * m;
    norm_scale(n, mean, centred, step, epsilon)
}

/// The rest of [`norm_row`]: from the squares about the mean to the
/// factor and the shift.
fn norm_scale(n: i64, mean: i16, squares: i64, step: f32, epsilon: f32) -> (i16, i16, u32) {
    // In real units: variance = squares / n * step^2 (`squares` is below
    // 2^38, exact enough as f32 for a standard deviation). The 32-bit
    // conversion when it fits: the 64-bit one is a software routine on
    // the board, and both give the nearest `f32`.
    let variance = crate::nn::quant::sum_to_f32(squares) / n as f32 * step * step;
    let rstd = 1.0 / libm::sqrtf(variance + epsilon);
    // value - mean (input units) times step * rstd gives the standardized
    // value; in units of 2^-11 that is times step * rstd * 2^11.
    let ratio = step * rstd * 2048.0;
    let shift = (14 - super::ceil_log2(ratio)).clamp(0, 30) as u32;
    let factor = libm::roundf(ratio * super::pow2(shift as i32)).clamp(0.0, 32767.0) as i16;
    (mean, factor, shift)
}

/// Apply a LayerNorm row with the constants of [`norm_constants`]
/// through the plans.
pub fn norm_apply(
    row: &[i16],
    mean: i16,
    factor: i16,
    shift: u32,
    plans: &[NormPlan],
    out: &mut [i16],
) {
    for (g, plan) in plans.iter().enumerate() {
        for j in 0..LANES {
            let c = g * LANES + j;
            let centred = row[c].saturating_sub(mean);
            let standardized = scale16(centred, factor, shift);
            let scaled = scale16(standardized, plan.factor[j], plan.shift);
            out[c] = scaled.saturating_add(plan.offset[j]);
        }
    }
}

/// `count` groups of eight outputs from `first` over `runs`, with their
/// epilogues, into `out`; see `lanes::linear` and `lanes::conv2d`.
pub fn groups(
    input: &[i16],
    weight: &LaneWeight<'_>,
    first: usize,
    count: usize,
    runs: &[Run],
    store: Store<'_>,
    out: &mut [i16],
) {
    for g in 0..count {
        let plan = &weight.plans[first + g];
        let mut sums = decode_lanes(&plan.image);
        for run in runs {
            let values = &input[run.input..run.input + run.len];
            for (k, &v) in values.iter().enumerate() {
                let base = ((first + g) * weight.per_output + run.weight + k) * LANES;
                // The 16-bit copy when there is one: the board reads that.
                match weight.wide {
                    Some(wide) => {
                        for (sum, &w) in sums.iter_mut().zip(&wide[base..base + LANES]) {
                            *sum += i64::from(v) * i64::from(w);
                        }
                    }
                    None => {
                        for (sum, &w) in sums.iter_mut().zip(&weight.data[base..base + LANES]) {
                            *sum += i64::from(v) * i64::from(w);
                        }
                    }
                }
            }
        }
        let values = epilogue(&sums, plan);
        self::store(&values, store, &mut out[g * LANES..(g + 1) * LANES]);
    }
}

/// One output pixel of a depthwise convolution over `taps`, every group
/// of eight channels through its plan, into `out`.
pub fn depthwise_pixel(
    input: &[i16],
    weights: &[i8],
    plans: &[GroupPlan],
    taps: &[Tap],
    store: Store<'_>,
    out: &mut [i16],
) {
    for (g, plan) in plans.iter().enumerate() {
        let mut sums = decode_lanes(&plan.image);
        for tap in taps {
            let c = g * LANES;
            let values = &input[tap.input + c..tap.input + c + LANES];
            let tap_weights = &weights[tap.weight + c..tap.weight + c + LANES];
            for ((sum, &v), &w) in sums.iter_mut().zip(values).zip(tap_weights) {
                *sum += i64::from(v) * i64::from(w);
            }
        }
        let values = epilogue(&sums, plan);
        self::store(&values, store, &mut out[g * LANES..(g + 1) * LANES]);
    }
}

/// `target += source`, saturating.
pub fn add(target: &mut [i16], source: &[i16]) {
    for (t, &s) in target.iter_mut().zip(source) {
        *t = t.saturating_add(s);
    }
}

/// `output = sat(input * factor >> shift)`.
pub fn rescale(input: &[i16], factor: i16, shift: u32, output: &mut [i16]) {
    for (o, &v) in output.iter_mut().zip(input) {
        *o = scale16(v, factor, shift);
    }
}

/// The per-channel maximum of two pixels of two rows.
pub fn max4(top: &[i16], bottom: &[i16], out: &mut [i16]) {
    let channels = out.len();
    for (c, o) in out.iter_mut().enumerate() {
        *o = top[c]
            .max(top[channels + c])
            .max(bottom[c])
            .max(bottom[channels + c]);
    }
}

/// The exact dot product of two rows.
pub fn dot(a: &[i16], b: &[i16]) -> i64 {
    a.iter()
        .zip(b)
        .map(|(&x, &y)| i64::from(x) * i64::from(y))
        .sum()
}

/// The attention's mixing; see `lanes::mix`.
pub fn mix(rows: &[i16], width: usize, weights: &[i16], factor: i16, shift: u32, out: &mut [i16]) {
    for (t, o) in out.iter_mut().enumerate() {
        let mut sum = 0i64;
        for (r, &w) in weights.iter().enumerate() {
            sum += i64::from(w) * i64::from(rows[r * width + t]);
        }
        // The sum in the accumulator, shifted to 16 bits first (by 15:
        // the weights' unit), then scaled.
        let narrow = sat16(sum >> 15);
        *o = scale16(narrow, factor, shift);
    }
}
