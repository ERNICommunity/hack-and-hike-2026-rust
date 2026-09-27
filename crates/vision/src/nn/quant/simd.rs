//! The vector unit of the ESP32-S3, for the integer kernels.
//!
//! The chip's *Processor Instruction Extensions* add eight 128-bit
//! registers and multiply-accumulate instructions that do eight 16-bit
//! products at once. The scalar kernels of `quant` cost about seventeen
//! clock cycles per product on this in-order core; the vector unit does
//! eight per instruction. This module is the only place in the crate that
//! uses `unsafe` (inline assembly), and it only exists on the board: on
//! any other target every function here reports "not available" and the
//! kernels use their scalar loops, which give the same bits.
//!
//! # What the kernels use
//!
//! - **Row sums** for the linear layers and full convolutions: the
//!   40-bit `ACCX` register accumulates all eight lanes of
//!   `ee.vmulas.s16.accx`, so a whole dot product of `i16` inputs with
//!   `i8` weights (widened to 16 bits on the fly) comes out as one exact
//!   integer. One assembly loop does a batch of outputs against the same
//!   input, so the per-output cost is a few instructions. See [`dots_i16`].
//! - **Eight channels of a depthwise convolution** at once: the eight
//!   40-bit lanes of `QACC` each accumulate one channel over the taps of
//!   an output pixel, and are read back exactly through memory. See
//!   [`DepthwisePixel`].
//!
//! Both are exact, so the results equal the scalar kernels' bit for bit;
//! the networks' outputs on the board equal the computer's bit for bit.
//!
//! # Alignment
//!
//! The 128-bit loads ignore the low four address bits and the 64-bit
//! loads the low three: a misaligned address silently reads the wrong
//! bytes. Every entry point therefore checks alignment and lengths and
//! returns `None` when they do not fit, and the kernels count such calls
//! in [`fallbacks`] so a slow layer can be traced to its buffer.
//!
//! # Registers
//!
//! The compiler never uses the vector registers or the accumulators, and
//! no other code in the firmware does, so the assembly blocks treat them
//! as their own; each assembly block leaves nothing it needs behind.
#![allow(unsafe_code)]

use core::sync::atomic::{AtomicUsize, Ordering};

use super::Tap;

/// Channels of a depthwise convolution computed together.
pub const LANES: usize = 8;

/// The most blocks of eight channels one depthwise assembly call sums:
/// 64 channels, whose raw accumulators fit a 512-byte array on the
/// stack.
pub const GROUP_BLOCKS: usize = 8;

/// One contiguous stretch of input that a group of outputs reads, for
/// [`group_sums_i16`]: where it starts in the input (in values), where
/// its weights start inside a filter (in weights), and how many values.
/// `repr(C)` because the assembly reads the triples.
#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
pub struct RunSpec {
    /// The offset of the first input value.
    pub input: usize,
    /// The offset of the first weight inside the filter.
    pub weight: usize,
    /// The number of values; even, at least two.
    pub len: usize,
}

/// The zero points of a block of eight channels, on a 16-byte boundary
/// for the vector load that subtracts them.
#[derive(Clone, Copy)]
#[repr(C, align(16))]
pub struct ZeroPoints(pub [i16; LANES]);

/// Kernel calls that could not use the vector unit although it exists.
static FALLBACKS: AtomicUsize = AtomicUsize::new(0);

/// Whether this build runs on a chip with the vector unit.
pub const fn available() -> bool {
    cfg!(target_arch = "xtensa")
}

/// How many kernel calls have used the scalar loops on a board that has
/// the vector unit: because of a misaligned buffer, a channel count that
/// is not a multiple of eight, an input mapping with a zero point, or an
/// 8-bit input to a full convolution.
pub fn fallbacks() -> usize {
    FALLBACKS.load(Ordering::Relaxed)
}

/// Record one such call; see [`fallbacks`].
pub(super) fn note_fallback() {
    if available() {
        FALLBACKS.fetch_add(1, Ordering::Relaxed);
    }
}

/// A value on a 16-byte boundary: an array a kernel builds on the stack
/// and hands to the vector unit, such as a padded weight.
#[repr(C, align(16))]
pub struct Aligned<T>(pub T);

/// Whether `slice` starts at a 16-byte boundary.
pub fn aligned16<T>(slice: &[T]) -> bool {
    slice.as_ptr().align_offset(16) == 0
}

/// Whether `slice` starts at an 8-byte boundary.
pub fn aligned8<T>(slice: &[T]) -> bool {
    slice.as_ptr().align_offset(8) == 0
}

/// The most outputs one call of [`dots_i16`] computes.
pub const DOT_BATCH: usize = 64;

/// Whether [`dots_i16`] accepts these operands: eight or more input
/// values in a multiple of eight, `input` on a 16-byte boundary,
/// `weights` on an 8-byte boundary with `count` filters of
/// `input.len()` weights `stride` apart inside it, `stride` a multiple of
/// eight (so every filter is 8-byte aligned), `count` at most
/// [`DOT_BATCH`].
pub fn dots_usable(input: &[i16], weights: &[i8], stride: usize, count: usize) -> bool {
    available()
        && !input.is_empty()
        && input.len().is_multiple_of(LANES)
        && stride >= input.len()
        && stride.is_multiple_of(LANES)
        && (1..=DOT_BATCH).contains(&count)
        && weights.len() >= (count - 1) * stride + input.len()
        && aligned16(input)
        && aligned8(weights)
}

/// The dot products of `input` with `count` filters: filter `k` is
/// `weights[k * stride..k * stride + input.len()]`. Exact, into
/// `sums[..count]`. Returns `false` and writes nothing when
/// [`dots_usable`] says no.
pub fn dots_i16(
    input: &[i16],
    weights: &[i8],
    stride: usize,
    count: usize,
    sums: &mut [i64; DOT_BATCH],
) -> bool {
    if !dots_usable(input, weights, stride, count) {
        return false;
    }
    #[cfg(target_arch = "xtensa")]
    {
        // The assembly stores each 40-bit sum as two words, which is the
        // memory layout of a `u64`; the sign extension is done in place.
        // SAFETY: `dots_usable` checked the alignment and that the weights
        // hold every filter the loop reads; `sums` has room for `count`
        // values and `i64` and `u64` share a layout; only registers the
        // compiler does not use are written.
        unsafe {
            arch::accx_dots_i16(
                input.as_ptr(),
                weights.as_ptr(),
                input.len(),
                stride,
                count,
                sums.as_mut_ptr().cast::<u64>(),
            );
        }
        for sum in &mut sums[..count] {
            *sum = sign_extend_40(*sum as u64);
        }
        true
    }
    #[cfg(not(target_arch = "xtensa"))]
    {
        let _ = sums;
        false
    }
}

/// The sums of `lanes.len()` groups of eight packed filters (see
/// `nn::pack`), from group `first_group`, over `runs`: for group `g` and
/// lane `j`, `sum over runs and k of input[run.input + k] *
/// w[8 g + j][run.weight + k]`, where filter `o` has `per_output`
/// weights. Exact 40-bit sums, group `b` in `lanes[b]` as
/// [`decode_lanes`] reads them. Returns `false` and writes nothing when
/// the operands do not fit: no vector unit, no runs or more groups than
/// [`GROUP_BLOCKS`], a run of odd or zero length or outside the input or
/// the filter, too few weights, or weights off an 8-byte boundary.
///
/// One input value is broadcast to the eight lanes and multiplied by the
/// eight filters' weights at that input, so a whole group of outputs
/// costs one loop with no per-output work; the input needs no
/// alignment.
pub fn group_sums_i16(
    input: &[i16],
    weights: &[i8],
    per_output: usize,
    first_group: usize,
    runs: &[RunSpec],
    lanes: &mut [Lanes],
) -> bool {
    let groups = lanes.len();
    let usable = available()
        && !runs.is_empty()
        && (1..=GROUP_BLOCKS).contains(&groups)
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
        && (first_group + groups)
            .checked_mul(per_output * LANES)
            .is_some_and(|end| end <= weights.len())
        && aligned8(weights);
    if !usable {
        return false;
    }
    #[cfg(target_arch = "xtensa")]
    {
        // SAFETY: every run was checked to lie inside the input and the
        // filters, the groups inside the weights, the weights on an
        // 8-byte boundary; `lanes` has one aligned slot per group; only
        // registers the compiler does not use are written.
        unsafe {
            arch::qacc_groups_i16(
                input.as_ptr(),
                weights.as_ptr().add(first_group * per_output * LANES),
                per_output * LANES,
                runs.as_ptr().cast(),
                runs.len(),
                lanes.as_mut_ptr(),
                groups,
            );
        }
        true
    }
    #[cfg(not(target_arch = "xtensa"))]
    {
        let _ = (input, first_group);
        false
    }
}

/// The dot product of `input` and one filter, exact, or `None` when the
/// operands do not fit the vector unit; see [`dots_usable`].
pub fn dot_i16(input: &[i16], weights: &[i8]) -> Option<i64> {
    let mut sums = [0i64; DOT_BATCH];
    dots_i16(input, weights, input.len(), 1, &mut sums).then_some(sums[0])
}

/// One output pixel of a depthwise convolution on the vector unit: the
/// input, the weights and the pixel's taps, checked once so that every
/// block of eight channels can be summed without further checks.
#[cfg_attr(not(target_arch = "xtensa"), allow(dead_code))]
pub struct DepthwisePixel<'a, T> {
    /// The channels-last input.
    input: &'a [T],
    /// The weights, `[tap][channel]`.
    weights: &'a [i8],
    /// The taps of the pixel.
    taps: &'a [Tap],
    /// The channel count of the input and the weights.
    channels: usize,
}

impl<'a, T> DepthwisePixel<'a, T> {
    /// Check the operands: the vector unit exists, there is at least one
    /// tap, `channels` is a multiple of eight, `input` and every tap's
    /// pixel start on a boundary of eight `T` (16 bytes for `i16`),
    /// `weights` and every tap's weights on an 8-byte boundary, and all
    /// `channels` values of every tap lie inside both slices.
    pub fn new(
        input: &'a [T],
        weights: &'a [i8],
        taps: &'a [Tap],
        channels: usize,
    ) -> Option<Self> {
        let align = LANES * core::mem::size_of::<T>();
        let usable = available()
            && !taps.is_empty()
            && channels.is_multiple_of(LANES)
            && input.as_ptr().align_offset(align) == 0
            && aligned8(weights)
            && taps.iter().all(|tap| {
                tap.input.is_multiple_of(LANES)
                    && tap.weight.is_multiple_of(LANES)
                    && tap.input + channels <= input.len()
                    && tap.weight + channels <= weights.len()
            });
        usable.then_some(Self {
            input,
            weights,
            taps,
            channels,
        })
    }

    /// Check that `channel` starts `blocks` blocks of eight inside the
    /// pixel.
    fn check_blocks(&self, channel: usize, blocks: usize) {
        assert!(
            blocks > 0
                && channel.is_multiple_of(LANES)
                && channel + blocks * LANES <= self.channels,
            "depthwise channel blocks"
        );
    }
}

impl DepthwisePixel<'_, i8> {
    /// The sums of consecutive blocks of eight channels from `channel`,
    /// one per entry of `zeros`, over the taps, each input value less its
    /// channel's zero point: `sum((input[tap.input + c] - zeros[c]) *
    /// weights[tap.weight + c])`. Block `b` lands in `lanes[b]`, as
    /// [`decode_lanes`] reads it.
    ///
    /// # Panics
    ///
    /// When `channel` is not a multiple of eight, the blocks do not fit
    /// the pixel, or `lanes` is shorter than `zeros`.
    pub fn sums(&self, zeros: &[ZeroPoints], channel: usize, lanes: &mut [Lanes]) {
        self.check_blocks(channel, zeros.len());
        assert!(lanes.len() >= zeros.len(), "depthwise lane storage");
        #[cfg(target_arch = "xtensa")]
        {
            // SAFETY: `new` checked that every tap's `channels` values from
            // its offsets lie inside both slices, so the blocks from
            // `channel` do; the pointers are aligned as the loads require;
            // `zeros` and `lanes` are aligned arrays the assembly reads and
            // writes whole, one entry per block.
            unsafe {
                arch::qacc_depthwise_i8(
                    self.input.as_ptr().add(channel),
                    zeros.as_ptr().cast(),
                    self.weights.as_ptr().add(channel),
                    self.taps.as_ptr().cast(),
                    self.taps.len(),
                    lanes.as_mut_ptr(),
                    zeros.len(),
                );
            }
        }
        #[cfg(not(target_arch = "xtensa"))]
        {
            let _ = lanes;
            unreachable!("no vector unit")
        }
    }
}

impl DepthwisePixel<'_, i16> {
    /// The sums of `lanes.len()` consecutive blocks of eight channels
    /// from `channel` over the taps: `sum(input[tap.input + c] *
    /// weights[tap.weight + c])`, block `b` in `lanes[b]`.
    ///
    /// # Panics
    ///
    /// When `channel` is not a multiple of eight or the blocks do not fit
    /// the pixel.
    pub fn sums(&self, channel: usize, lanes: &mut [Lanes]) {
        self.check_blocks(channel, lanes.len());
        #[cfg(target_arch = "xtensa")]
        {
            // SAFETY: as for the `i8` pixel; the input loads are 128-bit,
            // and `new` required 16-byte alignment for them.
            unsafe {
                arch::qacc_depthwise_i16(
                    self.input.as_ptr().add(channel),
                    self.weights.as_ptr().add(channel),
                    self.taps.as_ptr().cast(),
                    self.taps.len(),
                    lanes.as_mut_ptr(),
                    lanes.len(),
                );
            }
        }
        #[cfg(not(target_arch = "xtensa"))]
        {
            let _ = lanes;
            unreachable!("no vector unit")
        }
    }
}

/// `QACC` as the four store instructions write it, each into its own
/// 16-byte slot (the stores must not overlap: two stores to one word in
/// a row left stray bytes on the board): words 0..4 and word 4 hold the
/// 160 bits of `QACC_L` (lanes 0 to 3), words 8..12 and word 12 those of
/// `QACC_H` (lanes 4 to 7), each lane 40 bits, little-endian. Kept as
/// words so the decode loads words: as bytes, the compiler loaded them
/// one at a time, 200 instructions per decode.
#[derive(Clone, Copy)]
#[repr(C, align(16))]
pub struct Lanes(pub [u32; 16]);

/// A 40-bit value in the low bits of `raw`, sign-extended.
#[cfg_attr(not(target_arch = "xtensa"), allow(dead_code))]
#[inline]
fn sign_extend_40(raw: u64) -> i64 {
    let low = raw as u32;
    let high = (raw >> 32) as u8 as i8;
    (i64::from(high) << 32) | i64::from(low)
}

/// The eight 40-bit lane values of a stored `QACC`, sign-extended: each
/// 160-bit half is five words, and the lanes are cut out of them.
#[inline]
pub fn decode_lanes(words: &[u32; 16]) -> [i64; 8] {
    let mut sums = [0i64; LANES];
    for (half, base) in [0usize, 8].into_iter().enumerate() {
        let [w0, w1, w2, w3, w4] = [
            words[base],
            words[base + 1],
            words[base + 2],
            words[base + 3],
            words[base + 4],
        ];
        let lanes: [(u32, u8); 4] = [
            (w0, w1 as u8),
            ((w1 >> 8) | (w2 << 24), (w2 >> 8) as u8),
            ((w2 >> 16) | (w3 << 16), (w3 >> 16) as u8),
            ((w3 >> 24) | (w4 << 8), (w4 >> 24) as u8),
        ];
        for (i, (low, high)) in lanes.into_iter().enumerate() {
            sums[half * 4 + i] = (i64::from(high as i8) << 32) | i64::from(low);
        }
    }
    sums
}

#[cfg(target_arch = "xtensa")]
mod arch {
    //! The assembly. Vector registers `q0`..`q7`, `ACCX`, `QACC` and
    //! `SAR_BYTE` are used freely: nothing the compiler generates touches
    //! them.
    use core::arch::asm;

    use super::Lanes;

    /// `count` dot products of one input row (`len` values, a positive
    /// multiple of 8, 16-byte aligned) with filters `stride` weights
    /// apart (8-byte aligned), each result's raw 40 bits into `raw`.
    ///
    /// Per output: clear `ACCX`, sixteen products per loop iteration
    /// (two 128-bit loads of input, two 64-bit loads of weights into one
    /// register, a compare against zero for the sign bytes, a byte zip
    /// that turns the sixteen weights into two registers of sixteen-bit
    /// values, two multiply-accumulates), a tail of eight, then the two
    /// halves of `ACCX` stored as one 64-bit value.
    #[inline]
    pub unsafe fn accx_dots_i16(
        input: *const i16,
        weights: *const i8,
        len: usize,
        stride: usize,
        count: usize,
        raw: *mut u64,
    ) {
        let pairs = len / 16;
        let tail = (len % 16) / 8;
        let skip = stride - len;
        // SAFETY: the caller guarantees the reads and the room in `raw`;
        // only registers the compiler does not use are written.
        unsafe {
            asm!(
                "ee.zero.q q7",
                "4:",
                "ee.zero.accx",
                "mov {inp}, {input}",
                "mov {n}, {pairs}",
                "beqz {n}, 2f",
                "1:",
                "ee.vld.128.ip q0, {inp}, 16",
                "ee.vld.128.ip q1, {inp}, 16",
                "ee.vld.l.64.ip q2, {w}, 8",
                "ee.vld.h.64.ip q2, {w}, 8",
                "ee.vcmp.lt.s8 q3, q2, q7",
                "ee.vzip.8 q2, q3",
                "ee.vmulas.s16.accx q0, q2",
                "ee.vmulas.s16.accx q1, q3",
                "addi {n}, {n}, -1",
                "bnez {n}, 1b",
                "2:",
                "beqz {tail}, 3f",
                "ee.vld.128.ip q0, {inp}, 16",
                "ee.vld.l.64.ip q2, {w}, 8",
                "ee.vcmp.lt.s8 q3, q2, q7",
                "ee.vzip.8 q2, q3",
                "ee.vmulas.s16.accx q0, q2",
                "3:",
                "add {w}, {w}, {skip}",
                "rur.accx_0 {lo}",
                "rur.accx_1 {hi}",
                "s32i {lo}, {raw}, 0",
                "s32i {hi}, {raw}, 4",
                "addi {raw}, {raw}, 8",
                "addi {count}, {count}, -1",
                "bnez {count}, 4b",
                input = in(reg) input,
                w = inout(reg) weights => _,
                pairs = in(reg) pairs,
                tail = in(reg) tail,
                skip = in(reg) skip,
                count = inout(reg) count => _,
                raw = inout(reg) raw => _,
                inp = out(reg) _,
                n = out(reg) _,
                lo = out(reg) _,
                hi = out(reg) _,
                options(nostack),
            );
        }
    }

    /// `blocks` consecutive blocks of eight channels of a depthwise
    /// output pixel over `count` taps, each block into `QACC` and then
    /// stored to `out[block]`. `taps` points at `count` pairs of byte
    /// offsets (input, weights) relative to `input` and `weights`; each
    /// input address must be 8-byte aligned, and `zeros` (16 bytes per
    /// block) 16-byte aligned.
    #[inline]
    pub unsafe fn qacc_depthwise_i8(
        input: *const i8,
        zeros: *const i16,
        weights: *const i8,
        taps: *const usize,
        count: usize,
        out: *mut Lanes,
        blocks: usize,
    ) {
        // SAFETY: the caller guarantees the reads and the aligned `out`.
        unsafe {
            asm!(
                "ee.zero.q q7",
                "5:",
                "ee.zero.qacc",
                "ee.vld.128.ip q6, {zeros}, 16",
                "mov {t}, {taps}",
                "mov {n}, {count}",
                "1:",
                "l32i {ti}, {t}, 0",
                "l32i {tw}, {t}, 4",
                "addi {t}, {t}, 8",
                "add {ti}, {ti}, {inp}",
                "add {tw}, {tw}, {w}",
                "ee.vld.l.64.ip q0, {ti}, 0",
                "ee.vcmp.lt.s8 q1, q0, q7",
                "ee.vzip.8 q0, q1",
                "ee.vsubs.s16 q0, q0, q6",
                "ee.vld.l.64.ip q2, {tw}, 0",
                "ee.vcmp.lt.s8 q3, q2, q7",
                "ee.vzip.8 q2, q3",
                "ee.vmulas.s16.qacc q0, q2",
                "addi {n}, {n}, -1",
                "bnez {n}, 1b",
                "ee.st.qacc_l.l.128.ip {out}, 16",
                "ee.st.qacc_l.h.32.ip {out}, 16",
                "ee.st.qacc_h.l.128.ip {out}, 16",
                "ee.st.qacc_h.h.32.ip {out}, 16",
                "addi {inp}, {inp}, 8",
                "addi {w}, {w}, 8",
                "addi {blocks}, {blocks}, -1",
                "bnez {blocks}, 5b",
                "memw",
                inp = inout(reg) input => _,
                zeros = inout(reg) zeros => _,
                w = inout(reg) weights => _,
                taps = in(reg) taps,
                count = in(reg) count,
                out = inout(reg) out => _,
                blocks = inout(reg) blocks => _,
                t = out(reg) _,
                n = out(reg) _,
                ti = out(reg) _,
                tw = out(reg) _,
                options(nostack),
            );
        }
    }

    /// `groups` groups of eight packed filters over `count` runs, each
    /// group's eight sums into `QACC` and then stored to `out[group]`.
    /// `runs` points at `count` triples (input offset in values, weight
    /// offset in weights, length: even and positive); a group's weights
    /// are `stride` bytes (`per_output * 8`) and 8-byte aligned.
    ///
    /// Per run, two inputs per loop iteration: the eight weights of the
    /// group at each input are loaded (64 bits), widened to 16 bits by
    /// the compare-and-zip trick, and multiplied into the lanes by the
    /// input value broadcast in `q0`; the multiply-accumulate
    /// instruction itself loads and broadcasts the next input value.
    #[inline]
    pub unsafe fn qacc_groups_i16(
        input: *const i16,
        weights: *const i8,
        stride: usize,
        runs: *const usize,
        count: usize,
        out: *mut Lanes,
        groups: usize,
    ) {
        // SAFETY: the caller guarantees the reads and the aligned `out`.
        unsafe {
            asm!(
                "ee.zero.q q7",
                "5:",
                "ee.zero.qacc",
                "mov {r}, {runs}",
                "mov {rc}, {count}",
                "6:",
                "l32i {inp}, {r}, 0",
                "l32i {w}, {r}, 4",
                "l32i {n}, {r}, 8",
                "addi {r}, {r}, 12",
                "addx2 {inp}, {inp}, {input}",
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
                "ee.st.qacc_l.l.128.ip {out}, 16",
                "ee.st.qacc_l.h.32.ip {out}, 16",
                "ee.st.qacc_h.l.128.ip {out}, 16",
                "ee.st.qacc_h.h.32.ip {out}, 16",
                "add {wg}, {wg}, {stride}",
                "addi {groups}, {groups}, -1",
                "bnez {groups}, 5b",
                "memw",
                input = in(reg) input,
                wg = inout(reg) weights => _,
                stride = in(reg) stride,
                runs = in(reg) runs,
                count = in(reg) count,
                out = inout(reg) out => _,
                groups = inout(reg) groups => _,
                r = out(reg) _,
                rc = out(reg) _,
                inp = out(reg) _,
                w = out(reg) _,
                n = out(reg) _,
                options(nostack),
            );
        }
    }

    /// [`qacc_depthwise_i8`] for `i16` input without zero points; the
    /// tap offsets count values, and each input address must be 16-byte
    /// aligned.
    #[inline]
    pub unsafe fn qacc_depthwise_i16(
        input: *const i16,
        weights: *const i8,
        taps: *const usize,
        count: usize,
        out: *mut Lanes,
        blocks: usize,
    ) {
        // SAFETY: the caller guarantees the reads and the aligned `out`.
        unsafe {
            asm!(
                "ee.zero.q q7",
                "5:",
                "ee.zero.qacc",
                "mov {t}, {taps}",
                "mov {n}, {count}",
                "1:",
                "l32i {ti}, {t}, 0",
                "l32i {tw}, {t}, 4",
                "addi {t}, {t}, 8",
                "addx2 {ti}, {ti}, {inp}",
                "add {tw}, {tw}, {w}",
                "ee.vld.128.ip q0, {ti}, 0",
                "ee.vld.l.64.ip q2, {tw}, 0",
                "ee.vcmp.lt.s8 q3, q2, q7",
                "ee.vzip.8 q2, q3",
                "ee.vmulas.s16.qacc q0, q2",
                "addi {n}, {n}, -1",
                "bnez {n}, 1b",
                "ee.st.qacc_l.l.128.ip {out}, 16",
                "ee.st.qacc_l.h.32.ip {out}, 16",
                "ee.st.qacc_h.l.128.ip {out}, 16",
                "ee.st.qacc_h.h.32.ip {out}, 16",
                "addi {inp}, {inp}, 16",
                "addi {w}, {w}, 8",
                "addi {blocks}, {blocks}, -1",
                "bnez {blocks}, 5b",
                "memw",
                inp = inout(reg) input => _,
                w = inout(reg) weights => _,
                taps = in(reg) taps,
                count = in(reg) count,
                out = inout(reg) out => _,
                blocks = inout(reg) blocks => _,
                t = out(reg) _,
                n = out(reg) _,
                ti = out(reg) _,
                tw = out(reg) _,
                options(nostack),
            );
        }
    }
}
