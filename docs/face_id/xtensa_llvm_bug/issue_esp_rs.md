# Issue for esp-rs/rust

Filed with the "Regression" template.

## Title

1.98.1.0 miscompiles bytes of a static's address on Xtensa (1.97.0.0 and 1.98.0.0 failed with "Cannot select PCREL_WRAPPER")

## Body

### Code

I tried this code (`xtensa-esp32s3-none-elf`, `--release`, no LTO needed):

```rust
#![no_std]

pub static G: [u8; 8] = [0xAB, 0xCD, 0xEF, 0x01, 0, 0, 0, 0];

#[no_mangle]
pub fn low_byte() -> usize {
    G.as_ptr() as usize & 0xff
}
```

I expected to see this happen: `low_byte()` returns the low byte of `G`'s
address.

Instead, this happened: with 1.98.1.0 it compiles to

```text
	l32r	a8, .LCPI0_0          # a8 = &G
	l8ui	a2, a8, 0             # a2 = G[0]
```

and returns `G[0]`. Linked with `G` at `0x00500010` and run under
qemu-xtensa, it returns `0xab`; GCC's build of the same function returns
`0x10`. `(addr >> 8) & 0xff`, `addr >> 24` and `addr >> 16` go wrong the
same way (`l8ui a2, a8, 1`, `l8ui a2, a8, 3`, `l16ui a2, a8, 2`). With
1.97.0.0 the same code stops with:

```text
rustc-LLVM ERROR: Cannot select: 0xffff940b71b0: i32 = XtensaISD::PCREL_WRAPPER TargetConstantPool:i32<@_RNvCsg4kivnW5mOG_8low_byte1G = dso_local constant [8 x i8] c"\AB\CD\EF\01\00\00\00\00", align 1> 0
In function: low_byte
```

1.98.0.0 has the same LLVM commit as 1.97.0.0 (d092bf8f).

### Version it worked on

It most recently worked on: 1.93.0.0 (LLVM 20.1.1), which compiles it and
the three shifted forms correctly (`movi 255` / `and`, `srli` / `and`,
`extui`).

### Version with regression

`rustc --version --verbose`:

```text
rustc 1.98.1-nightly (183f762d6 2026-09-08) (1.98.1.0)
binary: rustc
commit-hash: 183f762d61935f539d2d68d52d80bca4bf8b7c76
commit-date: 2026-09-08
host: x86_64-unknown-linux-gnu
release: 1.98.1-nightly
LLVM version: 21.1.3
```

### Backtrace

None: 1.98.1.0 does not crash, it generates wrong code.

### Cause

- Upstream LLVM 21 (llvm/llvm-project 4154ada1d485, #136086) changed the
  Xtensa lowering of global addresses so that DAGCombine can narrow the
  literal-pool load to a byte load. Nothing can select that, hence the
  "Cannot select" with LLVM 21. Upstream report:
  llvm/llvm-project#227826.
- 1.98.1.0's LLVM (MabezDev/llvm-project 54f21f02) differs from
  1.98.0.0's (d092bf8f) by one cherry-pick, Espressif's 39a59933
  "[Xtensa] Fix ConstantPool lowering for aggregate constants". It targets
  the `[2 x float]` crashes in #277 and #282, but its fallback pattern
  makes every narrowed load of an address read memory at the address.
  Espressif report: espressif/llvm-project#139.
- The `[14 x i8]` string case in the comments on #277 (a global's
  address in an embassy task) may be this bug rather than the aggregate
  one.

We hit it through esp-hal 1.2 with `lto = "fat"`: SROA rebuilds a pointer
to the DMA channel metadata from its bytes, and with the fallback the
audio task reads its DMA metadata through a pointer whose low byte is the
metadata's first byte (checked by compiling the same LTO module with
Espressif LLVM 22.1.4, which has the same pattern).

Suggestion: replace the cherry-pick with the upstream approach for #277
and #282 (llvm/llvm-project#214974, which keeps DAGCombine from creating
the indexed constant-pool load), so that the address case is at least a
compile error again until LLVM has a fix.

Reproduced with the x86_64 Linux build of 1.98.1.0 under qemu-x86_64:
the release has no aarch64 Linux archive, unlike 1.97.0.0 and 1.98.0.0.
