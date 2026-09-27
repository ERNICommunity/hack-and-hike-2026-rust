//! The assembly of the lane kernels. Vector registers `q0`..`q7`, `QACC`
//! and `SAR` are used freely: nothing the compiler generates touches
//! them. Every function checks its operands and returns `false` when
//! they do not fit the instructions' alignment rules, so the caller can
//! fall back to the scalar model.
#![allow(unsafe_code)]

use core::arch::asm;

use super::{GroupPlan, LANES, LaneWeight, NormPlan, Run, Store, Tap};

/// Whether `slice` starts at a 16-byte boundary.
fn aligned16<T>(slice: &[T]) -> bool {
    slice.as_ptr().align_offset(16) == 0
}

/// Whether `slice` starts at an 8-byte boundary.
fn aligned8<T>(slice: &[T]) -> bool {
    slice.as_ptr().align_offset(8) == 0
}

/// `count` groups from `first` over `runs` with their epilogues into
/// `out`; see `groups_epilogue`.
pub fn groups(
    input: &[i16],
    weight: &LaneWeight<'_>,
    first: usize,
    count: usize,
    runs: &[Run],
    store: Store<'_>,
    out: &mut [i16],
) -> bool {
    let per_output = weight.per_output;
    let usable = count > 0
        && !runs.is_empty()
        && runs.iter().all(|run| {
            run.len >= 2
                && run.len.is_multiple_of(2)
                && run
                    .input
                    .checked_add(run.len)
                    .is_some_and(|end| end <= input.len())
                && run
                    .weight
                    .checked_add(run.len)
                    .is_some_and(|end| end <= per_output)
        })
        && (first + count)
            .checked_mul(per_output * LANES)
            .is_some_and(|end| end <= weight.data.len())
        && first + count <= weight.plans.len()
        && aligned8(weight.data)
        && aligned16(out)
        && out.len() == count * LANES;
    if !usable {
        return false;
    }
    let limit = Constants([GELU_LIMIT; 8]);
    let args = GroupArgs {
        input: input.as_ptr(),
        weights: weight.data[first * per_output * LANES..].as_ptr(),
        stride: per_output * LANES,
        runs: runs.as_ptr().cast(),
        run_count: runs.len(),
        plans: weight.plans[first..].as_ptr(),
        groups: count,
        out: out.as_mut_ptr(),
        limit: limit.0.as_ptr(),
    };
    // SAFETY: every run lies inside the input and the filters, the groups
    // inside the weights and the plans; `out` is aligned with room for
    // the groups; only registers the compiler does not use are written.
    unsafe {
        match store {
            Store::Write => qacc_groups::<0>(&args),
            Store::Add => qacc_groups::<1>(&args),
            Store::Relu => qacc_groups::<2>(&args),
            Store::Gelu(_) => qacc_groups::<3>(&args),
        }
    }
    true
}

/// The largest GELU input, in units of `2^-10`: 16.
const GELU_LIMIT: i16 = 16384;

/// The operands of [`qacc_groups`], read by the assembly at fixed
/// offsets (the register file is too small to hold them all).
#[repr(C)]
struct GroupArgs {
    /// The input values (offset 0).
    input: *const i16,
    /// The first group's weights (4).
    weights: *const i8,
    /// Bytes per group of weights (8).
    stride: usize,
    /// The runs, as triples of words (12).
    runs: *const usize,
    /// How many runs (16).
    run_count: usize,
    /// The first group's plan (20).
    plans: *const GroupPlan,
    /// How many groups (24).
    groups: usize,
    /// The first group's outputs (28).
    out: *mut i16,
    /// Eight copies of the GELU limit (32).
    limit: *const i16,
}

/// The epilogue of one group, as assembly template lines: the
/// accumulator holds the sums (bias image included), `{plan}` points at
/// the group's plan; the result is in `q1`.
macro_rules! epilogue {
    () => {
        concat!(
            // First shift: sums to 16 bits, rounded by the image's half.
            "l32i {t}, {plan}, 112\n",
            "ee.srcmb.s16.qacc q1, {t}, 0\n",
            // Plus half, times factor, second shift, plus offset.
            "addi {t}, {plan}, 64\n",
            "ee.vld.128.ip q2, {t}, 16\n",
            "ee.vld.128.ip q3, {t}, 16\n",
            "ee.vld.128.ip q4, {t}, 0\n",
            "ee.vadds.s16 q1, q1, q2\n",
            "ee.zero.qacc\n",
            "ee.vmulas.s16.qacc q1, q3\n",
            "l32i {t}, {plan}, 116\n",
            "ee.srcmb.s16.qacc q1, {t}, 0\n",
            "ee.vadds.s16 q1, q1, q4\n",
        )
    };
}

/// The store of a finished group in `q1` to `{out}`, by mode: 0 write,
/// 1 add, 2 relu, 3 clamp to the GELU range (`q6` holds the limit, `q7`
/// zero).
macro_rules! store_mode {
    (0) => {
        "ee.vst.128.ip q1, {out}, 16"
    };
    (1) => {
        concat!(
            "ee.vld.128.ip q2, {out}, 0\n",
            "ee.vadds.s16 q1, q1, q2\n",
            "ee.vst.128.ip q1, {out}, 16",
        )
    };
    (2) => {
        concat!("ee.vmax.s16 q1, q1, q7\n", "ee.vst.128.ip q1, {out}, 16",)
    };
    (3) => {
        concat!(
            "ee.vmin.s16 q1, q1, q6\n",
            "ee.vsubs.s16 q2, q7, q6\n",
            "ee.vmax.s16 q1, q1, q2\n",
            "ee.vst.128.ip q1, {out}, 16",
        )
    };
}

/// `count` groups over `runs` into `out`, `MODE` 0 write, 1 add, 2 relu,
/// 3 clamp to the GELU range. Per group: load the bias image into the
/// accumulator, accumulate every run (two inputs per iteration: the
/// eight weights at each input widened from 8 to 16 bits, the input
/// broadcast by the multiply-accumulate's own load), then the epilogue.
#[inline]
unsafe fn qacc_groups<const MODE: u8>(args: &GroupArgs) {
    macro_rules! body {
        ($store:expr) => {
            asm!(
                "ee.zero.q q7",
                "l32i {t}, {args}, 32",
                "ee.vld.128.ip q6, {t}, 0",
                "l32i {wg}, {args}, 4",
                "l32i {plan}, {args}, 20",
                "l32i {groups}, {args}, 24",
                "l32i {out}, {args}, 28",
                "5:",
                // The bias image into the accumulator.
                "mov {t}, {plan}",
                "ee.ld.qacc_l.l.128.ip {t}, 16",
                "ee.ld.qacc_l.h.32.ip {t}, 16",
                "ee.ld.qacc_h.l.128.ip {t}, 16",
                "ee.ld.qacc_h.h.32.ip {t}, 0",
                "l32i {r}, {args}, 12",
                "l32i {rc}, {args}, 16",
                "6:",
                "l32i {inp}, {r}, 0",
                "l32i {w}, {r}, 4",
                "l32i {n}, {r}, 8",
                "addi {r}, {r}, 12",
                "l32i {t}, {args}, 0",
                "addx2 {inp}, {inp}, {t}",
                "addx8 {w}, {w}, {wg}",
                "srli {n}, {n}, 1",
                "addi {n}, {n}, -1",
                "ee.vldbc.16.ip q0, {inp}, 2",
                "beqz {n}, 2f",
                "1:",
                "ee.vld.l.64.ip q2, {w}, 8",
                "ee.vld.l.64.ip q4, {w}, 8",
                "ee.vcmp.lt.s8 q3, q2, q7",
                "ee.vcmp.lt.s8 q5, q4, q7",
                "ee.vzip.8 q2, q3",
                "ee.vzip.8 q4, q5",
                "ee.vmulas.s16.qacc.ldbc.incp q0, {inp}, q0, q2",
                "ee.vmulas.s16.qacc.ldbc.incp q0, {inp}, q0, q4",
                "addi {n}, {n}, -1",
                "bnez {n}, 1b",
                "2:",
                "ee.vld.l.64.ip q2, {w}, 8",
                "ee.vld.l.64.ip q4, {w}, 8",
                "ee.vcmp.lt.s8 q3, q2, q7",
                "ee.vcmp.lt.s8 q5, q4, q7",
                "ee.vzip.8 q2, q3",
                "ee.vzip.8 q4, q5",
                "ee.vmulas.s16.qacc.ldbc.incp q0, {inp}, q0, q2",
                "ee.vmulas.s16.qacc q0, q4",
                "addi {rc}, {rc}, -1",
                "bnez {rc}, 6b",
                epilogue!(),
                $store,
                "l32i {t}, {args}, 8",
                "add {wg}, {wg}, {t}",
                "addi {plan}, {plan}, 128",
                "addi {groups}, {groups}, -1",
                "bnez {groups}, 5b",
                "memw",
                args = in(reg) args as *const GroupArgs,
                wg = out(reg) _,
                plan = out(reg) _,
                groups = out(reg) _,
                out = out(reg) _,
                r = out(reg) _,
                rc = out(reg) _,
                inp = out(reg) _,
                w = out(reg) _,
                n = out(reg) _,
                t = out(reg) _,
                options(nostack),
            )
        };
    }
    // SAFETY: the caller checked the operands.
    unsafe {
        match MODE {
            0 => body!(store_mode!(0)),
            1 => body!(store_mode!(1)),
            2 => body!(store_mode!(2)),
            _ => body!(store_mode!(3)),
        }
    }
}

/// One output pixel of a depthwise convolution: every group of eight
/// channels over `taps`, with the epilogues, into `out`.
pub fn depthwise_pixel(
    input: &[i16],
    weights: &[i8],
    plans: &[GroupPlan],
    taps: &[Tap],
    store: Store<'_>,
    out: &mut [i16],
) -> bool {
    let channels = plans.len() * LANES;
    let usable = !taps.is_empty()
        && !plans.is_empty()
        && aligned16(input)
        && aligned8(weights)
        && aligned16(out)
        && out.len() == channels
        && taps.iter().all(|tap| {
            tap.input.is_multiple_of(LANES)
                && tap.weight.is_multiple_of(LANES)
                && tap.input + channels <= input.len()
                && tap.weight + channels <= weights.len()
        });
    if !usable {
        return false;
    }
    let limit = Constants([GELU_LIMIT; 8]);
    let args = DepthwiseArgs {
        input: input.as_ptr(),
        weights: weights.as_ptr(),
        taps: taps.as_ptr().cast(),
        count: taps.len(),
        plans: plans.as_ptr(),
        groups: plans.len(),
        out: out.as_mut_ptr(),
        limit: limit.0.as_ptr(),
    };
    // SAFETY: every tap's `channels` values lie inside both slices at
    // aligned offsets; `out` has room for every group.
    unsafe {
        match store {
            Store::Write => qacc_depthwise::<0>(&args),
            Store::Add => qacc_depthwise::<1>(&args),
            Store::Relu => qacc_depthwise::<2>(&args),
            Store::Gelu(_) => qacc_depthwise::<3>(&args),
        }
    }
    true
}

/// The operands of [`qacc_depthwise`], read at fixed offsets.
#[repr(C)]
struct DepthwiseArgs {
    /// The input, channel 0 of pixel 0 (offset 0).
    input: *const i16,
    /// The weights, `[tap][channel]` (4).
    weights: *const i8,
    /// The taps, as pairs of words (8).
    taps: *const usize,
    /// How many taps (12).
    count: usize,
    /// The first group's plan (16).
    plans: *const GroupPlan,
    /// How many groups (20).
    groups: usize,
    /// The pixel's outputs (24).
    out: *mut i16,
    /// Eight copies of the GELU limit (28).
    limit: *const i16,
}

/// Every group of a depthwise output pixel over `count` taps (pairs of
/// offsets in values into the input and in weights), each into `QACC`
/// from its bias image, then the epilogue.
#[inline]
unsafe fn qacc_depthwise<const MODE: u8>(args: &DepthwiseArgs) {
    macro_rules! body {
        ($store:expr) => {
            asm!(
                "ee.zero.q q7",
                "l32i {t}, {args}, 28",
                "ee.vld.128.ip q6, {t}, 0",
                "l32i {inp}, {args}, 0",
                "l32i {w}, {args}, 4",
                "l32i {plan}, {args}, 16",
                "l32i {groups}, {args}, 20",
                "l32i {out}, {args}, 24",
                "5:",
                "mov {t}, {plan}",
                "ee.ld.qacc_l.l.128.ip {t}, 16",
                "ee.ld.qacc_l.h.32.ip {t}, 16",
                "ee.ld.qacc_h.l.128.ip {t}, 16",
                "ee.ld.qacc_h.h.32.ip {t}, 0",
                "l32i {tp}, {args}, 8",
                "l32i {n}, {args}, 12",
                "1:",
                "l32i {ti}, {tp}, 0",
                "l32i {tw}, {tp}, 4",
                "addi {tp}, {tp}, 8",
                "addx2 {ti}, {ti}, {inp}",
                "add {tw}, {tw}, {w}",
                "ee.vld.128.ip q0, {ti}, 0",
                "ee.vld.l.64.ip q2, {tw}, 0",
                "ee.vcmp.lt.s8 q3, q2, q7",
                "ee.vzip.8 q2, q3",
                "ee.vmulas.s16.qacc q0, q2",
                "addi {n}, {n}, -1",
                "bnez {n}, 1b",
                epilogue!(),
                $store,
                "addi {inp}, {inp}, 16",
                "addi {w}, {w}, 8",
                "addi {plan}, {plan}, 128",
                "addi {groups}, {groups}, -1",
                "bnez {groups}, 5b",
                "memw",
                args = in(reg) args as *const DepthwiseArgs,
                inp = out(reg) _,
                w = out(reg) _,
                plan = out(reg) _,
                groups = out(reg) _,
                out = out(reg) _,
                tp = out(reg) _,
                n = out(reg) _,
                ti = out(reg) _,
                tw = out(reg) _,
                t = out(reg) _,
                options(nostack),
            )
        };
    }
    // SAFETY: the caller checked the operands.
    unsafe {
        match MODE {
            0 => body!(store_mode!(0)),
            1 => body!(store_mode!(1)),
            2 => body!(store_mode!(2)),
            _ => body!(store_mode!(3)),
        }
    }
}

/// One LayerNorm row through the plans; see `model::norm_apply`.
pub fn norm_row(
    row: &[i16],
    mean: i16,
    factor: i16,
    shift: u32,
    plans: &[NormPlan],
    out: &mut [i16],
) -> bool {
    let channels = plans.len() * LANES;
    if row.len() != channels || out.len() != channels || !aligned16(row) || !aligned16(out) {
        return false;
    }
    let constants = Constants([mean, factor, 0, 0, 0, 0, 0, 0]);
    // SAFETY: the row, the output and every plan are aligned and have
    // `channels` values; the constants are an aligned stack value.
    unsafe {
        asm!(
            "ee.vldbc.16 q6, {constants}",
            "addi {t}, {constants}, 2",
            "ee.vldbc.16 q5, {t}",
            "1:",
            "ee.vld.128.ip q0, {row}, 16",
            "ee.vsubs.s16 q0, q0, q6",
            "ee.zero.qacc",
            "ee.vmulas.s16.qacc q0, q5",
            "ee.srcmb.s16.qacc q1, {shift}, 0",
            "ee.vld.128.ip q2, {plan}, 16",
            "ee.vld.128.ip q3, {plan}, 16",
            "l32i {t}, {plan}, 0",
            "addi {plan}, {plan}, 16",
            "ee.zero.qacc",
            "ee.vmulas.s16.qacc q1, q2",
            "ee.srcmb.s16.qacc q1, {t}, 0",
            "ee.vadds.s16 q1, q1, q3",
            "ee.vst.128.ip q1, {out}, 16",
            "addi {n}, {n}, -1",
            "bnez {n}, 1b",
            "memw",
            row = inout(reg) row.as_ptr() => _,
            out = inout(reg) out.as_mut_ptr() => _,
            plan = inout(reg) plans.as_ptr() => _,
            n = inout(reg) plans.len() => _,
            shift = in(reg) shift,
            constants = in(reg) constants.0.as_ptr(),
            t = out(reg) _,
            options(nostack),
        );
    }
    true
}

/// Eight 16-bit values on a 16-byte boundary.
#[repr(C, align(16))]
struct Constants([i16; 8]);

/// The exact dot product of two aligned `i16` rows of a multiple of
/// eight values, through the 40-bit `ACCX`.
pub fn dot(a: &[i16], b: &[i16]) -> Option<i64> {
    let len = a.len();
    if len < LANES || !len.is_multiple_of(LANES) || !aligned16(a) || !aligned16(b) {
        return None;
    }
    let (low, high): (u32, u32);
    // SAFETY: both rows are aligned and hold `len` values.
    unsafe {
        asm!(
            "ee.zero.accx",
            "1:",
            "ee.vld.128.ip q0, {a}, 16",
            "ee.vld.128.ip q1, {b}, 16",
            "ee.vmulas.s16.accx q0, q1",
            "addi {n}, {n}, -1",
            "bnez {n}, 1b",
            "rur.accx_0 {lo}",
            "rur.accx_1 {hi}",
            a = inout(reg) a.as_ptr() => _,
            b = inout(reg) b.as_ptr() => _,
            n = inout(reg) len / LANES => _,
            lo = out(reg) low,
            hi = out(reg) high,
            options(nostack, readonly),
        );
    }
    let raw = (u64::from(high & 0xFF) << 32) | u64::from(low);
    Some((i64::from((high & 0xFF) as u8 as i8) << 32) | i64::from(raw as u32))
}

/// The attention's mixing; see `lanes::mix`.
pub fn mix(
    rows: &[i16],
    width: usize,
    weights: &[i16],
    factor: i16,
    shift: u32,
    out: &mut [i16],
) -> bool {
    if weights.is_empty() || !aligned16(rows) || !aligned16(out) {
        return false;
    }
    let constants = Constants([factor; 8]);
    // SAFETY: `rows` holds `weights.len()` aligned rows of `width`
    // values, `out` `width` values; the constants are aligned.
    unsafe {
        asm!(
            "ee.vld.128.ip q5, {constants}, 0",
            "movi {s1}, 15",
            "2:",
            "ee.zero.qacc",
            "mov {r}, {rows}",
            "mov {w}, {weights}",
            "mov {n}, {count}",
            "1:",
            "ee.vldbc.16.ip q0, {w}, 2",
            "ee.vld.128.xp q1, {r}, {stride}",
            "ee.vmulas.s16.qacc q0, q1",
            "addi {n}, {n}, -1",
            "bnez {n}, 1b",
            "ee.srcmb.s16.qacc q1, {s1}, 0",
            "ee.zero.qacc",
            "ee.vmulas.s16.qacc q1, q5",
            "ee.srcmb.s16.qacc q1, {shift}, 0",
            "ee.vst.128.ip q1, {out}, 16",
            "addi {rows}, {rows}, 16",
            "addi {blocks}, {blocks}, -1",
            "bnez {blocks}, 2b",
            "memw",
            rows = inout(reg) rows.as_ptr() => _,
            stride = in(reg) width * 2,
            weights = in(reg) weights.as_ptr(),
            count = in(reg) weights.len(),
            shift = in(reg) shift,
            out = inout(reg) out.as_mut_ptr() => _,
            blocks = inout(reg) width / LANES => _,
            constants = in(reg) constants.0.as_ptr(),
            r = out(reg) _,
            w = out(reg) _,
            n = out(reg) _,
            s1 = out(reg) _,
            options(nostack),
        );
    }
    true
}

/// `target += source`, saturating, sixteen bytes at a time.
pub fn add(target: &mut [i16], source: &[i16]) -> bool {
    let len = target.len();
    if len < LANES || !len.is_multiple_of(LANES) || !aligned16(target) || !aligned16(source) {
        return false;
    }
    // SAFETY: both slices are aligned and hold `len` values, a multiple
    // of eight.
    unsafe {
        asm!(
            "1:",
            "ee.vld.128.ip q0, {t}, 0",
            "ee.vld.128.ip q1, {s}, 16",
            "ee.vadds.s16 q0, q0, q1",
            "ee.vst.128.ip q0, {t}, 16",
            "addi {n}, {n}, -1",
            "bnez {n}, 1b",
            "memw",
            t = inout(reg) target.as_mut_ptr() => _,
            s = inout(reg) source.as_ptr() => _,
            n = inout(reg) len / LANES => _,
            options(nostack),
        );
    }
    true
}

/// `output = sat(input * factor >> shift)`, sixteen bytes at a time.
pub fn rescale(input: &[i16], factor: i16, shift: u32, output: &mut [i16]) -> bool {
    let len = input.len();
    if len < LANES || !len.is_multiple_of(LANES) || !aligned16(input) || !aligned16(output) {
        return false;
    }
    let constants = Constants([factor; 8]);
    // SAFETY: both slices are aligned and hold `len` values, a multiple
    // of eight; the constants are an aligned stack value.
    unsafe {
        asm!(
            "ee.vld.128.ip q5, {constants}, 0",
            "1:",
            "ee.vld.128.ip q0, {i}, 16",
            "ee.zero.qacc",
            "ee.vmulas.s16.qacc q0, q5",
            "ee.srcmb.s16.qacc q1, {shift}, 0",
            "ee.vst.128.ip q1, {o}, 16",
            "addi {n}, {n}, -1",
            "bnez {n}, 1b",
            "memw",
            i = inout(reg) input.as_ptr() => _,
            o = inout(reg) output.as_mut_ptr() => _,
            n = inout(reg) len / LANES => _,
            shift = in(reg) shift,
            constants = in(reg) constants.0.as_ptr(),
            options(nostack),
        );
    }
    true
}

/// The maximum of two pixels of two rows (`top` and `bottom` each hold
/// two adjacent pixels of `channels` values), per channel.
pub fn max4(top: &[i16], bottom: &[i16], out: &mut [i16]) -> bool {
    let channels = out.len();
    if channels < LANES
        || !channels.is_multiple_of(LANES)
        || top.len() != 2 * channels
        || bottom.len() != 2 * channels
        || !aligned16(top)
        || !aligned16(bottom)
        || !aligned16(out)
    {
        return false;
    }
    // SAFETY: all slices are aligned with the stated lengths.
    unsafe {
        asm!(
            "1:",
            "ee.vld.128.ip q0, {a}, 16",
            "ee.vld.128.ip q1, {b}, 16",
            "ee.vld.128.ip q2, {c}, 16",
            "ee.vld.128.ip q3, {d}, 16",
            "ee.vmax.s16 q0, q0, q1",
            "ee.vmax.s16 q2, q2, q3",
            "ee.vmax.s16 q0, q0, q2",
            "ee.vst.128.ip q0, {o}, 16",
            "addi {n}, {n}, -1",
            "bnez {n}, 1b",
            "memw",
            a = inout(reg) top.as_ptr() => _,
            b = inout(reg) top.as_ptr().add(channels) => _,
            c = inout(reg) bottom.as_ptr() => _,
            d = inout(reg) bottom.as_ptr().add(channels) => _,
            o = inout(reg) out.as_mut_ptr() => _,
            n = inout(reg) channels / LANES => _,
            options(nostack),
        );
    }
    true
}
