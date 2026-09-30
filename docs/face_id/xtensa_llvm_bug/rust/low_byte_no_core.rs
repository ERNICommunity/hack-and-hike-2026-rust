// src/lib.rs without `core`, so that no standard library has to be built
// for the target. It links against runtime/harness.s, which defines `g`.
//
//   RUSTC_BOOTSTRAP=1 rustc +esp --target xtensa-esp32s3-none-elf \
//     -C opt-level=3 -C panic=abort --emit asm -o - low_byte_no_core.rs
//
// esp-rs Rust 1.93.0.0:            movi a8, 255 / l32r a9, .LCPI0_0 / and a2, a9, a8
// esp-rs Rust 1.97.0.0 / 1.98.0.0: LLVM ERROR: Cannot select (PCREL_WRAPPER)
// esp-rs Rust 1.98.1.0:            l32r a8, .LCPI0_0 / l8ui a2, a8, 0
#![feature(no_core, lang_items)]
#![allow(internal_features)]
#![no_core]
#![crate_type = "lib"]

#[lang = "pointee_sized"] pub trait PointeeSized {}
#[lang = "meta_sized"] pub trait MetaSized: PointeeSized {}
#[lang = "sized"] pub trait Sized: MetaSized {}
#[lang = "copy"] pub trait Copy {}
impl Copy for usize {}
#[lang = "bitand"]
pub trait BitAnd<Rhs = Self> { type Output; fn bitand(self, rhs: Rhs) -> Self::Output; }
impl BitAnd for usize { type Output = usize; fn bitand(self, rhs: usize) -> usize { self & rhs } }

unsafe extern "C" { #[link_name = "g"] static G: [u8; 8]; }

#[no_mangle]
pub fn low_byte() -> usize {
    (&raw const G) as usize & 0xff
}
