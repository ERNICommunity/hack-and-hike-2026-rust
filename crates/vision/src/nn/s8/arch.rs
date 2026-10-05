//! The assembly of the 8-bit kernels. Vector registers `q0`..`q7` and
//! `QACC` are used freely: nothing the compiler generates touches them.
#![allow(unsafe_code)]

use core::arch::asm;

use super::{BATCH_BYTES, Depthwise, LANES, Plan, Pointwise, Store};

/// Whether `slice` starts at a 16-byte boundary.
fn aligned16<T>(slice: &[T]) -> bool {
    slice.as_ptr().align_offset(16) == 0
}

/// The plan's image into the accumulator, through `{t}`.
macro_rules! load_image {
    () => {
        concat!(
            "mov {t}, {plan}\n",
            "ee.ld.qacc_l.l.128.ip {t}, 16\n",
            "ee.ld.qacc_l.h.32.ip {t}, 16\n",
            "ee.ld.qacc_h.l.128.ip {t}, 16\n",
            "ee.ld.qacc_h.h.32.ip {t}, 0\n",
        )
    };
}

/// The sixteen sums to `i8` in `q2`, by `{shift}`, then through the
/// plan's PReLU or not (`q7` holds zeros). The PReLU: the non-negative
/// part doubled `positive` times (saturating); the negative part times
/// the slopes from the PReLU image, shifted (rounded by the image's
/// half); the two added, one of them zero in every lane.
macro_rules! epilogue {
    (plain) => {
        "ee.srcmb.s8.qacc q2, {shift}, 0\n"
    };
    (prelu) => {
        concat!(
            "ee.srcmb.s8.qacc q2, {shift}, 0\n",
            "ee.vmin.s8 q3, q2, q7\n",
            "ee.vmax.s8 q4, q2, q7\n",
            "l32i {t}, {plan}, 144\n",
            "beqz {t}, 4f\n",
            "ee.vadds.s8 q4, q4, q4\n",
            "4:\n",
            "addi {t}, {plan}, 64\n",
            "ee.ld.qacc_l.l.128.ip {t}, 16\n",
            "ee.ld.qacc_l.h.32.ip {t}, 16\n",
            "ee.ld.qacc_h.l.128.ip {t}, 16\n",
            "ee.ld.qacc_h.h.32.ip {t}, 16\n",
            "ee.vld.128.ip q5, {t}, 0\n",
            "ee.vmulas.s8.qacc q3, q5\n",
            "l32i {t}, {plan}, 148\n",
            "ee.srcmb.s8.qacc q3, {t}, 0\n",
            "ee.vadds.s8 q2, q4, q3\n",
        )
    };
}

/// The finished sixteen values in `q2` to `{out}`, which then steps by
/// `{stride}`: written, or added to what is there (saturating).
macro_rules! store {
    (write) => {
        "ee.vst.128.xp q2, {out}, {stride}\n"
    };
    (add) => {
        concat!(
            "ee.vld.128.ip q3, {out}, 0\n",
            "ee.vadds.s8 q2, q2, q3\n",
            "ee.vst.128.xp q2, {out}, {stride}\n",
        )
    };
}

/// What the assembly of one 1x1 group reads, by byte offset.
#[repr(C)]
struct GroupArgs {
    /// 0: the first input pixel.
    input: *const i8,
    /// 4: the group's weights, `[input][16]`.
    weights: *const i8,
    /// 8: the group's plan.
    plan: *const Plan,
    /// 12: pixels to compute, at least one.
    pixels: usize,
    /// 16: `(input channels - 2) / 2`: the inner loop's passes.
    pairs: usize,
    /// 20: the group's sixteen outputs of the first pixel.
    output: *mut i8,
    /// 24: bytes from one pixel's outputs to the next.
    stride: usize,
}

/// One 1x1 group over its pixels.
///
/// Per pixel: the plan's image into the accumulator; the first input
/// value broadcast to the sixteen lanes of `q0` and the first sixteen
/// weights into `q1`; then, in a zero-overhead `loopgtz`, each
/// multiply-accumulate loads the next input value into `q0` and steps
/// the input pointer, and a load fetches the next weights. After the
/// last channel the input pointer is at the next pixel. The epilogue
/// and the store follow; the store steps by the output stride.
macro_rules! pointwise_group {
    ($args:expr, $epilogue:tt, $store:tt) => {
        asm!(
            "ee.zero.q q7",
            "l32i {inp}, {args}, 0",
            "l32i {plan}, {args}, 8",
            "l32i {n}, {args}, 12",
            "l32i {out}, {args}, 20",
            "l32i {stride}, {args}, 24",
            "l32i {shift}, {plan}, 152",
            "1:",
            load_image!(),
            "l32i {w}, {args}, 4",
            "l32i {t}, {args}, 16",
            "ee.vldbc.8.ip q0, {inp}, 1",
            "ee.vld.128.ip q1, {w}, 16",
            "loopgtz {t}, 2f",
            "ee.vmulas.s8.qacc.ldbc.incp q0, {inp}, q0, q1",
            "ee.vld.128.ip q1, {w}, 16",
            "ee.vmulas.s8.qacc.ldbc.incp q0, {inp}, q0, q1",
            "ee.vld.128.ip q1, {w}, 16",
            "2:",
            "ee.vmulas.s8.qacc.ldbc.incp q0, {inp}, q0, q1",
            "ee.vld.128.ip q1, {w}, 16",
            "ee.vmulas.s8.qacc q0, q1",
            epilogue!($epilogue),
            store!($store),
            "addi {n}, {n}, -1",
            "bnez {n}, 1b",
            "memw",
            args = in(reg) $args as *const GroupArgs,
            inp = out(reg) _,
            plan = out(reg) _,
            n = out(reg) _,
            out = out(reg) _,
            stride = out(reg) _,
            shift = out(reg) _,
            w = out(reg) _,
            t = out(reg) _,
            options(nostack),
        )
    };
}

/// [`super::pointwise`] on the vector unit; `false` when an operand does
/// not fit the instructions' rules.
pub fn pointwise(
    layer: &Pointwise<'_>,
    prelu: bool,
    store: Store,
    input: &[i8],
    output: &mut [i8],
) -> bool {
    let (channels, outputs) = (layer.input, layer.output());
    if !(aligned16(layer.weights) && aligned16(output) && aligned16(layer.plans)) {
        return false;
    }
    let pixels = input.len() / channels;
    let batch = (BATCH_BYTES / channels).max(1);
    for first in (0..pixels).step_by(batch) {
        let count = batch.min(pixels - first);
        let pixels_in = &input[first * channels..(first + count) * channels];
        for (group, plan) in layer.plans.iter().enumerate() {
            let weights = &layer.weights[group * channels * LANES..(group + 1) * channels * LANES];
            let start = first * outputs + group * LANES;
            let out = &mut output[start..start + (count - 1) * outputs + LANES];
            let args = GroupArgs {
                input: pixels_in.as_ptr(),
                weights: weights.as_ptr(),
                plan,
                pixels: count,
                pairs: (channels - 2) / 2,
                output: out.as_mut_ptr(),
                stride: outputs,
            };
            // SAFETY: the pixels lie inside the input, the group's
            // weights inside the weights, every pixel's sixteen outputs
            // inside the output; the weights, the plan and every output
            // are 16-byte aligned (`outputs` is a multiple of 16); only
            // registers the compiler does not use are written.
            unsafe {
                match (prelu, store) {
                    (false, Store::Write) => pointwise_group!(&args, plain, write),
                    (false, Store::Add) => pointwise_group!(&args, plain, add),
                    (true, Store::Write) => pointwise_group!(&args, prelu, write),
                    (true, Store::Add) => pointwise_group!(&args, prelu, add),
                }
            }
        }
    }
    true
}

/// One tap of a depthwise output pixel: where its input pixel starts, and
/// the byte offset of its sixteen weights inside a group's 144.
#[derive(Clone, Copy)]
#[repr(C)]
struct Tap {
    /// The input pixel's first channel.
    input: *const i8,
    /// `(3 ky + kx) * 16`.
    weight: usize,
}

/// What the assembly of one depthwise output pixel reads, by byte offset.
#[repr(C)]
struct PixelArgs {
    /// 0: the taps that lie inside the image.
    taps: *const Tap,
    /// 4: how many, at least one.
    count: usize,
    /// 8: the first group's weights, `[3][3][16]`.
    weights: *const i8,
    /// 12: the first group's plan.
    plans: *const Plan,
    /// 16: groups of sixteen channels, at least one.
    groups: usize,
    /// 20: the output pixel's first channel.
    output: *mut i8,
}

/// Every group of one depthwise output pixel: per group, the plan's
/// image into the accumulator, then per tap sixteen inputs times sixteen
/// weights lane by lane, the epilogue, and a store; then the next
/// group's weights (144 bytes on), plan (224 on) and channels (16 on).
macro_rules! depthwise_pixel {
    ($args:expr, $epilogue:tt) => {
        asm!(
            "ee.zero.q q7",
            "l32i {groups}, {args}, 16",
            "l32i {plan}, {args}, 12",
            "l32i {wg}, {args}, 8",
            "l32i {out}, {args}, 20",
            "movi {goff}, 0",
            "1:",
            "l32i {shift}, {plan}, 152",
            load_image!(),
            "l32i {r}, {args}, 0",
            "l32i {n}, {args}, 4",
            "loopgtz {n}, 2f",
            "l32i {inp}, {r}, 0",
            "l32i {t}, {r}, 4",
            "addi {r}, {r}, 8",
            "add {inp}, {inp}, {goff}",
            "add {t}, {t}, {wg}",
            "ee.vld.128.ip q0, {inp}, 0",
            "ee.vld.128.ip q1, {t}, 0",
            "ee.vmulas.s8.qacc q0, q1",
            "2:",
            epilogue!($epilogue),
            "ee.vst.128.ip q2, {out}, 16",
            "addi {wg}, {wg}, 72",
            "addi {wg}, {wg}, 72",
            "addi {plan}, {plan}, 112",
            "addi {plan}, {plan}, 112",
            "addi {goff}, {goff}, 16",
            "addi {groups}, {groups}, -1",
            "bnez {groups}, 1b",
            "memw",
            args = in(reg) $args as *const PixelArgs,
            groups = out(reg) _,
            plan = out(reg) _,
            wg = out(reg) _,
            out = out(reg) _,
            shift = out(reg) _,
            goff = out(reg) _,
            r = out(reg) _,
            n = out(reg) _,
            inp = out(reg) _,
            t = out(reg) _,
            options(nostack),
        )
    };
}

/// [`super::depthwise_row`] on the vector unit; `false` when an operand
/// does not fit the instructions' rules.
pub fn depthwise_row(
    layer: &Depthwise<'_>,
    prelu: bool,
    rows: [Option<&[i8]>; 3],
    width: usize,
    output: &mut [i8],
) -> bool {
    let channels = layer.channels();
    let fits = aligned16(layer.weights)
        && aligned16(layer.plans)
        && aligned16(output)
        && rows.iter().flatten().all(|row| aligned16(row));
    if !fits || core::mem::size_of::<Plan>() != super::PLAN_BYTES {
        return false;
    }
    for (ox, out) in output.chunks_exact_mut(channels).enumerate() {
        let centre = ox * layer.stride;
        let mut taps = [Tap {
            input: core::ptr::null(),
            weight: 0,
        }; 9];
        let mut count = 0;
        for (ky, row) in rows.iter().enumerate() {
            let Some(row) = row else { continue };
            for kx in 0..3 {
                let Some(x) = (centre + kx).checked_sub(1).filter(|&x| x < width) else {
                    continue;
                };
                taps[count] = Tap {
                    input: row[x * channels..].as_ptr(),
                    weight: (ky * 3 + kx) * LANES,
                };
                count += 1;
            }
        }
        if count == 0 {
            return false;
        }
        let args = PixelArgs {
            taps: taps.as_ptr(),
            count,
            weights: layer.weights.as_ptr(),
            plans: layer.plans.as_ptr(),
            groups: channels / LANES,
            output: out.as_mut_ptr(),
        };
        // SAFETY: every tap's pixel lies inside its row and has
        // `channels` values, every group's weights and plan lie inside
        // the layer's, the output pixel has `channels` values; all are
        // 16-byte aligned; only registers the compiler does not use are
        // written.
        unsafe {
            if prelu {
                depthwise_pixel!(&args, prelu);
            } else {
                depthwise_pixel!(&args, plain);
            }
        }
    }
    true
}
