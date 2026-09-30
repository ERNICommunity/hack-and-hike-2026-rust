# Xtensa LLVM bug: bytes of an address

Taking a byte or a half of a global's address (`addr & 0xff`,
`(addr >> 8) & 0xff`, `addr >> 24`, `addr >> 16`) breaks the Xtensa code
generator of LLVM since LLVM 21:

- LLVM 19 and 20 (esp-rs Rust up to 1.93.0.0) compile it correctly;
- upstream LLVM since 4154ada1d485 (#136086), Espressif LLVM 21.1.3 and
  esp-rs Rust 1.97.0.0 and 1.98.0.0 stop with
  `Cannot select: ... XtensaISD::PCREL_WRAPPER`;
- Espressif LLVM 22.1.4 and esp-rs Rust 1.98.1.0 compile it without a
  diagnostic, to a load of the bytes stored at the address (wrong code).

This is the crash behind "`lto = "fat"` does not build" in
[performance.md](../performance.md). Fat LTO inlines esp-hal's DMA
metadata into the audio task, where SROA rebuilds a pointer from its
bytes. The builds without LTO compile, so they contain no instance of
the pattern.

## Files

| File | What |
| --- | --- |
| [low_byte.ll](low_byte.ll) | the reduced IR: six cases with `RUN`/`CHECK` lines for `llvm/test/CodeGen/Xtensa` |
| [low_byte.c](low_byte.c) | the first case in C |
| [rust/](rust/) | the first case in Rust: a `no_std` crate, and `low_byte_no_core.rs` without `core` |
| [runtime/](runtime/) | a harness that runs `low_byte()` under `qemu-xtensa` and exits with its result |
| [setup.sh](setup.sh) | installs Espressif LLVM 20.1.1, 21.1.3 and 22.1.4, `qemu-xtensa`, FileCheck, and esp-rs Rust 1.93.0.0 and 1.98.1.0 |
| [build_upstream_llc.sh](build_upstream_llc.sh) | builds `llc` from upstream main with Xtensa (10 minutes) |
| [reproduce.sh](reproduce.sh) | runs everything and prints what each toolchain does |
| [issue_llvm_upstream.md](issue_llvm_upstream.md) | issue for llvm/llvm-project: the crash, a regression from #136086 |
| [issue_espressif_llvm.md](issue_espressif_llvm.md) | issue for espressif/llvm-project: the wrong code from 39a59933, in their form |
| [issue_esp_rs.md](issue_esp_rs.md) | issue for esp-rs/rust: the wrong code in 1.98.1.0, in the regression template |

## Running it

```sh
./setup.sh
./build_upstream_llc.sh   # optional
./reproduce.sh
```

The last lines prove the wrong code: `g` is at `0x00500010` and `g[0]` is
`0xAB`.

```text
== Running low_byte() under qemu-xtensa: g is at 0x00500010, g[0] = 0xAB
  gcc          returns 0x10: correct
  llvm-20.1.1  returns 0x10: correct
  llvm-22.1.4  returns 0xab: WRONG, expected 0x10
  rust-1.98.1  returns 0xab: WRONG, expected 0x10
```

## Filing

In this order, because each report links the previous one:

1. llvm/llvm-project: [issue_llvm_upstream.md](issue_llvm_upstream.md).
   Then a comment on llvm/llvm-project#214974 that links it.
2. espressif/llvm-project: [issue_espressif_llvm.md](issue_espressif_llvm.md),
   with `xtensa_llvm_bug.zip` attached.
3. esp-rs/rust: [issue_esp_rs.md](issue_esp_rs.md). Then a comment on
   esp-rs/rust#277 that links it.

Replace each `TODO` with the number of the issue it names.

## For this repository

- Keep `lto` off, and do not combine esp-rs Rust 1.98.1.0 with
  `lto = "fat"`: that build compiles, but the audio task reads its DMA
  channel metadata through a wrong pointer.
- 1.98.1.0 differs from 1.98.0.0's LLVM only in the fallback that turns
  the crash into wrong code. A firmware that builds with 1.98.0.0 (or
  the installed 1.97.0.0) has no instance of the pattern, so check a
  1.98.1.0 build with one of them.

Afterwards: if a maintainer asks you to test a fix, ./build_upstream_llc.sh <COMMIT> followed by ./reproduce.sh does it.
