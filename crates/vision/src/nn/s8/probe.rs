//! Single instructions of the 8-bit mode on known values, to check on the
//! board what the kernels assume about them: where each lane sits in the
//! accumulator, how the shift rounds, how far the broadcast load steps,
//! and what a lane does when its sum outgrows 20 bits. Each function
//! returns what the hardware produced; the caller compares.
#![allow(unsafe_code)]

use core::arch::asm;

use super::{LANES, encode_lanes};

/// Sixteen bytes on a 16-byte boundary, as vector loads and stores need.
#[derive(Clone, Copy)]
#[repr(C, align(16))]
struct Vector([i8; LANES]);

/// One `ee.vmulas.s8.qacc` of `1` (broadcast) times `[1, 2, ..., 16]`
/// from a zero accumulator: the accumulator's 64 bytes as
/// `ee.st.qacc_*` stores them (to compare with
/// [`encode_lanes`]`([1, 2, ..., 16])`), and the sixteen lanes after
/// `ee.srcmb.s8.qacc` with no shift (`[1, 2, ..., 16]` when lane `i` is
/// output `i`).
pub fn lane_layout() -> ([u32; 16], [i8; LANES]) {
    let one = Vector([1; LANES]);
    let mut ramp = Vector([0; LANES]);
    for (i, value) in ramp.0.iter_mut().enumerate() {
        *value = i as i8 + 1;
    }
    let mut raw = [0u32; 16];
    let mut lanes = Vector([0; LANES]);
    // SAFETY: every address is a local on a 16-byte boundary with room
    // for what is loaded or stored; only vector registers and QACC are
    // written besides the named ones.
    unsafe {
        asm!(
            "ee.zero.qacc",
            "ee.vldbc.8 q0, {one}",
            "ee.vld.128.ip q1, {ramp}, 0",
            "ee.vmulas.s8.qacc q0, q1",
            "mov {t}, {raw}",
            "ee.st.qacc_l.l.128.ip {t}, 16",
            "ee.st.qacc_l.h.32.ip {t}, 16",
            "ee.st.qacc_h.l.128.ip {t}, 16",
            "ee.st.qacc_h.h.32.ip {t}, 0",
            "movi {t}, 0",
            "ee.srcmb.s8.qacc q2, {t}, 0",
            "ee.vst.128.ip q2, {lanes}, 0",
            "memw",
            one = in(reg) one.0.as_ptr(),
            ramp = inout(reg) ramp.0.as_ptr() => _,
            raw = in(reg) raw.as_mut_ptr(),
            lanes = inout(reg) lanes.0.as_mut_ptr() => _,
            t = out(reg) _,
            options(nostack),
        );
    }
    (raw, lanes.0)
}

/// The accumulator loaded with `values` through [`encode_lanes`], then
/// `ee.srcmb.s8.qacc` by `shift`: what the kernels' plans and epilogue
/// rely on. Without a shift, small values come back as they are; with
/// one, the result shows whether the shift truncates or rounds, and how
/// negative values go.
pub fn shift_lanes(values: &[i32; LANES], shift: u32) -> [i8; LANES] {
    let image = super::Plan {
        image: encode_lanes(values),
        ..super::Plan::ZERO
    };
    let mut lanes = Vector([0; LANES]);
    // SAFETY: as in `lane_layout`.
    unsafe {
        asm!(
            "mov {t}, {image}",
            "ee.ld.qacc_l.l.128.ip {t}, 16",
            "ee.ld.qacc_l.h.32.ip {t}, 16",
            "ee.ld.qacc_h.l.128.ip {t}, 16",
            "ee.ld.qacc_h.h.32.ip {t}, 0",
            "ee.srcmb.s8.qacc q2, {shift}, 0",
            "ee.vst.128.ip q2, {lanes}, 0",
            "memw",
            image = in(reg) image.image.as_ptr(),
            shift = in(reg) shift,
            lanes = inout(reg) lanes.0.as_mut_ptr() => _,
            t = out(reg) _,
            options(nostack),
        );
    }
    lanes.0
}

/// `ee.vldbc.8.ip` of the first byte of `[10, 11, 12, ...]`, then one
/// `ee.vmulas.s8.qacc.ldbc.incp`: how many bytes the pointer moved in
/// all (2 when each load steps one byte), and the sixteen lanes of the
/// second broadcast (all 11 when it loaded the next byte).
pub fn broadcast_step() -> (usize, [i8; LANES]) {
    let mut data = Vector([0; LANES]);
    for (i, value) in data.0.iter_mut().enumerate() {
        *value = 10 + i as i8;
    }
    let zero = Vector([0; LANES]);
    let mut lanes = Vector([0; LANES]);
    let start = data.0.as_ptr();
    let end: *const i8;
    // SAFETY: as in `lane_layout`; the two loads read the first two
    // bytes of `data`.
    unsafe {
        asm!(
            "ee.zero.qacc",
            "ee.vld.128.ip q1, {zero}, 0",
            "ee.vldbc.8.ip q0, {p}, 1",
            "ee.vmulas.s8.qacc.ldbc.incp q0, {p}, q0, q1",
            "ee.vst.128.ip q0, {lanes}, 0",
            "memw",
            zero = inout(reg) zero.0.as_ptr() => _,
            p = inout(reg) start => end,
            lanes = inout(reg) lanes.0.as_mut_ptr() => _,
            options(nostack),
        );
    }
    (end as usize - start as usize, lanes.0)
}

/// `count` products of `127 * 127` (16,129 each) into every lane from
/// zero, then `ee.srcmb.s8.qacc` by 13. With lanes wide enough the result
/// is `count * 16129 >> 13` (78 for 40); a 20-bit lane that wraps gives
/// the wrapped sum shifted (-50 for 40); one that saturates gives
/// `524287 >> 13` (63).
pub fn overflow(count: u32) -> [i8; LANES] {
    let full = Vector([127; LANES]);
    let mut lanes = Vector([0; LANES]);
    // SAFETY: as in `lane_layout`.
    unsafe {
        asm!(
            "ee.zero.qacc",
            "ee.vld.128.ip q0, {full}, 0",
            "beqz {n}, 2f",
            "1:",
            "ee.vmulas.s8.qacc q0, q0",
            "addi {n}, {n}, -1",
            "bnez {n}, 1b",
            "2:",
            "movi {t}, 13",
            "ee.srcmb.s8.qacc q2, {t}, 0",
            "ee.vst.128.ip q2, {lanes}, 0",
            "memw",
            full = inout(reg) full.0.as_ptr() => _,
            n = inout(reg) count => _,
            lanes = inout(reg) lanes.0.as_mut_ptr() => _,
            t = out(reg) _,
            options(nostack),
        );
    }
    lanes.0
}
