//! The low byte of a static's address, `and (ptrtoint @G), 255` in LLVM IR.
//!
//!     cargo rustc --release -- --emit asm
//!
//! esp-rs Rust 1.93.0.0 (LLVM 20.1.1): correct (`movi 255` / `and`).
//! esp-rs Rust 1.97.0.0 and 1.98.0.0: rustc-LLVM ERROR: Cannot select:
//!   i32 = XtensaISD::PCREL_WRAPPER TargetConstantPool:i32<@...G ...>
//!   at every opt-level, without LTO.
//! esp-rs Rust 1.98.1.0 (LLVM with 54f21f02 "[Xtensa] Fix ConstantPool
//!   lowering for aggregate constants"): compiles, to `l32r` + `l8ui`,
//!   and returns G[0] (0xAB) instead of the address's low byte. Checked
//!   with `low_byte_no_core.rs`, the same function without `core`.
#![no_std]

pub static G: [u8; 8] = [0xAB, 0xCD, 0xEF, 0x01, 0, 0, 0, 0];

#[no_mangle]
pub fn low_byte() -> usize {
    G.as_ptr() as usize & 0xff
}
