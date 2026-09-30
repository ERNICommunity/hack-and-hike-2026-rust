# Issue for espressif/llvm-project

Filed with Espressif's bug report form; each section below is one field.

## Title

[Xtensa] Wrong code for bytes of a global's address since 39a59933 (reads memory at the global)

## Checklist

All three boxes apply: no existing report of the wrong code (the related
crash reports are listed under Additional context), no configuration
involved, tested with esp-22.1.4_20260825, the latest release.

## How often does this bug occurs?

always

## Expected behavior

A byte or a half of a global's address is computed from the address:

```c
extern char g[8];
unsigned low_byte(void) { return (unsigned)g & 0xff; }
```

```text
 movi a8, 255
 l32r a9, .LCPI0_0          # .literal .LCPI0_0, g
 and a2, a9, a8
```

(`llc` from esp-20.1.1_20250829 and esp-19.1.2_20250312 generates this
for the IR under "Steps to reproduce"; GCC generates `l32r` /
`extui a2, a8, 0, 8` for the C.)

## Actual behavior (suspected bug)

esp-22.1.4_20260825 compiles it without a diagnostic to a load from the
global itself:

```text
 l32r a8, .LCPI0_0          # a8 = &g
 l8ui a2, a8, 0             # a2 = g[0], not (&g & 0xff)
```

With `g` at `0x00500010` and `g[0] = 0xAB`, this returns `0xAB` under
qemu-xtensa, where GCC's build returns `0x10`.

Every byte and both halves of the address are affected:

| C | esp-22.1.4 |
| --- | --- |
| `(unsigned)&g & 0xff` | `l8ui a2, a8, 0` |
| `((unsigned)&g >> 8) & 0xff` | `l8ui a2, a8, 1` |
| `(unsigned)&g >> 24` | `l8ui a2, a8, 3` |
| `(unsigned)&g >> 16` | `l16ui a2, a8, 2` |
| `(int)&g >> 16` | `l16si a2, a8, 2` |
| `((unsigned)&g >> 24) == 0x3f` | `l8ui a8, a8, 3`, then the compare |

In esp-21.1.3_20260408 and upstream main the same code stops with
`Cannot select: XtensaISD::PCREL_WRAPPER` instead. Upstream report:
<https://github.com/llvm/llvm-project/issues/227826>

## Error logs or terminal output

There is no error; the output above is the bug. For comparison, esp-21.1.3:

```text
$ llc -mtriple=xtensa -mcpu=esp32s3 -O2 low_byte.ll -o -
LLVM ERROR: Cannot select: 0xaaaad2ae6900: i32 = XtensaISD::PCREL_WRAPPER TargetConstantPool:i32<@g = external global [8 x i8]> 0
In function: low_byte
```

## Steps to reproduce the behavior

1. Save the smallest case as `low_byte.ll` (all six cases, with
   `RUN`/`CHECK` lines, are in the upstream issue and in the attached zip):

   ```llvm
   @g = external global [8 x i8]

   define i32 @low_byte() {
     %p = ptrtoint ptr @g to i32
     %r = and i32 %p, 255
     ret i32 %r
   }
   ```

2. `llc -mtriple=xtensa -mcpu=esp32s3 -O2 low_byte.ll -o -` with
   esp-22.1.4_20260825, or
   `clang --target=xtensa-esp-elf -mcpu=esp32s3 -O2 -S low_byte.c -o -`.
3. The output has `l8ui` from the literal's contents, as shown above. The
   six-case `low_byte.ll` passes FileCheck with esp-19.1.2 and esp-20.1.1
   and fails with esp-21.1.3 and esp-22.1.4.

`reproduce.sh` in the zip runs all of it, including the qemu-xtensa run.

## Project release version

esp-22.1.4_20260825 (also esp-rs Rust 1.98.1.0, which carries 39a59933)

## System architecture

ARM 64-bit (Apple M1/M2, Raspberry Pi 4/5)

## Operating system

Linux

## Operating system version

Debian 13 (trixie), aarch64 (a dev container)

## Shell

Bash

## Additional context

**Cause.** Upstream 4154ada1d485 (#136086, in LLVM 21) changed what
`XtensaISD::PCREL_WRAPPER` means. Before, `LowerGlobalAddress` returned
the bare wrapper, and `def : Pat<(Xtensa_pcrel_wrapper tconstpool:$in), (L32R tconstpool:$in)>`
selected it as the literal's *value*. Since then `LowerGlobalAddress`
returns a generic `load` of the wrapper, so the wrapper is the literal's
*address*, and the pattern was removed. DAGCombine (`ReduceLoadWidth`)
narrows that load to an i8/i16 load at the literal (+0..3), which leaves a
bare wrapper as the address. 39a59933 ("[Xtensa] Fix ConstantPool
lowering for aggregate constants") restores the old pattern with
`AddedComplexity = -1`. Under the new meaning it is right only for your
aggregate constants, whose literal holds the aggregate's address. For
every other literal it yields the contents, and the narrowed load then
reads memory at the global.

**How we found it.** In Rust firmware for the ESP32-S3 with `lto = "fat"`,
SROA splits a pointer to esp-hal 1.2's DMA channel metadata into bytes and
joins them again (`or disjoint (upper bytes), zext i8 (ptrtoint (ptr @...INFO to i8))`).
esp-21.1.3 stops with the error; esp-22.1.4 builds the same module and
rebuilds the pointer with `INFO`'s first byte as its low byte, then reads
fields through it (`l32r a8, <INFO>` / `l8ui a8, a8, 0` /
`or a8, a7, a8` / `l8ui a8, a8, 12`).

**Related reports.** 39a59933 targets the aggregate-constant crash in
esp-rs/rust#277 and #282 (`[2 x float]` pool entries from a select of FP
constants); a comment on #277 points to it as the fix. The `[14 x i8]` string case in a comment on #277 is a
global's address in an embassy task and may be this bug instead. Upstream,
llvm/llvm-project#214974 (an open PR) fixes the FP-select crash by overriding
`reduceSelectOfFPConstantLoads`, without the bare-wrapper pattern; that
would also cover `constantpool_aggregate.ll`, as far as we can see.

**Suggestion.** Drop the fallback pattern, take the #214974 approach for
the aggregate case, and keep DAGCombine from narrowing loads whose base is
a `PCREL_WRAPPER` (for example in `shouldReduceLoadWidth`).

Attached: `xtensa_llvm_bug.zip` (the IR test, the C and Rust versions,
the qemu harness, `setup.sh` and `reproduce.sh`).
