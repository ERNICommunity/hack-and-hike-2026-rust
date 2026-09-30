# Issue for llvm/llvm-project

No template; paste the title and the body.

## Title

[Xtensa] Cannot select PCREL_WRAPPER when DAGCombine narrows the load of a global's address (regression from #136086)

## Body

Taking a byte or a half of a global's address crashes instruction
selection:

```llvm
@g = external global [8 x i8]

define i32 @low_byte() {
  %p = ptrtoint ptr @g to i32
  %r = and i32 %p, 255
  ret i32 %r
}
```

```console
$ llc -mtriple=xtensa low_byte.ll -o -
LLVM ERROR: Cannot select: t8: i32 = <<Unknown Target Node #491>> TargetConstantPool:i32<@g = external global [8 x i8]> 0
In function: low_byte
...
2.	Running pass 'Xtensa DAG->DAG Pattern Instruction Selection' on function '@low_byte'
```

Node #491 is `XtensaISD::PCREL_WRAPPER`, from `getAddrPCRel`. (Its name
is not printed because `XtensaSubtarget` creates a plain
`SelectionDAGTargetInfo` instead of `XtensaSelectionDAGInfo`, so the
generated node names are never used. That is a separate small bug.)

Expected, as LLVM 20 and 8f01edfa112f (the parent of 4154ada1d485) compile it:

```text
	movi	a8, 255
	l32r	a9, .LCPI0_0
	and	a2, a9, a8
```

Reproduced on main at 10fd0a4ec7ff (2026-09-29), in a Release build with
assertions and only Xtensa enabled; also at `-O0` and with
`-mcpu=esp32s3`. Compiler Explorer does not build the experimental Xtensa
target, so there is no link. Found through Rust firmware for the ESP32-S3
(esp-hal with `lto = "fat"`), reduced with llvm-reduce.

### Affected patterns

Every byte and both halves of the address fail the same way. Other masks
(1, 3, 7, 15, 127, 240), `zext(trunc to i16)` and `sext(trunc to i8)`
compile.

| IR | C |
| --- | --- |
| `and %p, 255` | `(unsigned)&g & 0xff` |
| `and (lshr %p, 8), 255` | `((unsigned)&g >> 8) & 0xff` |
| `lshr %p, 24` | `(unsigned)&g >> 24` |
| `lshr %p, 16` | `(unsigned)&g >> 16` |
| `ashr %p, 16` | `(int)&g >> 16` |
| `icmp eq (lshr %p, 24), 63` | `((unsigned)&g >> 24) == 0x3f` |

The test below has all six, with `RUN`/`CHECK` lines (no
`l8ui`/`l16ui`/`l16si` after the `l32r`). It passes with LLVM 19 and 20
(Espressif's esp-19.1.2 and esp-20.1.1 builds) and at 8f01edfa112f, and
fails at 4154ada1d485 and on main.

<details><summary>low_byte.ll</summary>

```llvm
; Bytes and halves of a global's address. The address is an i32 load from
; the literal pool, which DAGCombine must not narrow to an i8/i16 load:
; Xtensa can only read a literal with L32R, and the address of a literal
; (XtensaISD::PCREL_WRAPPER) is not selectable on its own.
; Correct code has no l8ui/l16ui/l16si, only L32R and register operations.
;
; RUN: llc -mtriple=xtensa -O2 < %s | FileCheck %s
; RUN: llc -mtriple=xtensa -mcpu=esp32s3 -O2 < %s | FileCheck %s
; RUN: llc -mtriple=xtensa -mcpu=esp32s3 -O0 < %s | FileCheck %s

@g = external global [8 x i8]

; (unsigned)&g & 0xff
define i32 @low_byte() {
; CHECK-LABEL: low_byte:
; CHECK:       l32r
; CHECK-NOT:   {{l8ui|l16ui|l16si}}
; CHECK:       ret
  %p = ptrtoint ptr @g to i32
  %r = and i32 %p, 255
  ret i32 %r
}

; ((unsigned)&g >> 8) & 0xff
define i32 @second_byte() {
; CHECK-LABEL: second_byte:
; CHECK:       l32r
; CHECK-NOT:   {{l8ui|l16ui|l16si}}
; CHECK:       ret
  %p = ptrtoint ptr @g to i32
  %s = lshr i32 %p, 8
  %r = and i32 %s, 255
  ret i32 %r
}

; (unsigned)&g >> 24
define i32 @top_byte() {
; CHECK-LABEL: top_byte:
; CHECK:       l32r
; CHECK-NOT:   {{l8ui|l16ui|l16si}}
; CHECK:       ret
  %p = ptrtoint ptr @g to i32
  %r = lshr i32 %p, 24
  ret i32 %r
}

; (unsigned)&g >> 16
define i32 @high_half() {
; CHECK-LABEL: high_half:
; CHECK:       l32r
; CHECK-NOT:   {{l8ui|l16ui|l16si}}
; CHECK:       ret
  %p = ptrtoint ptr @g to i32
  %r = lshr i32 %p, 16
  ret i32 %r
}

; (int)&g >> 16
define i32 @high_half_signed() {
; CHECK-LABEL: high_half_signed:
; CHECK:       l32r
; CHECK-NOT:   {{l8ui|l16ui|l16si}}
; CHECK:       ret
  %p = ptrtoint ptr @g to i32
  %r = ashr i32 %p, 16
  ret i32 %r
}

; ((unsigned)&g >> 24) == 0x3f, a memory region check
define i1 @in_region() {
; CHECK-LABEL: in_region:
; CHECK:       l32r
; CHECK-NOT:   {{l8ui|l16ui|l16si}}
; CHECK:       ret
  %p = ptrtoint ptr @g to i32
  %s = lshr i32 %p, 24
  %r = icmp eq i32 %s, 63
  ret i1 %r
}
```

</details>

### What happens

`-debug-only=isel,dagcombine` on main, shortened:

```text
Legalized selection DAG: %bb.0 'low_byte:'
        t8: i32 = <<Unknown Target Node #491>> TargetConstantPool:i32<@g = external global [8 x i8]> 0
      t10: i32,ch = load<(load (s32) from constant-pool)> t0, t8, poison:i32
    t3: i32 = and t10, Constant:i32<255>

Combining: t3: i32 = and t10, Constant:i32<255>
 ... into: t12: i32,ch = load<(load (s8) from constant-pool, align 4), zext from i8> t0, t8, poison:i32

ISEL: Starting selection on root node: t12: i32,ch = load<(load (s8) from constant-pool, align 4), zext from i8> t0, t8, poison:i32
  Morphed node: t12: i32,ch = L8UI<Mem:(load (s8) from constant-pool, align 4)> t8, TargetConstant:i32<0>, t0
ISEL: Starting selection on root node: t8: i32 = <<Unknown Target Node #491>> TargetConstantPool:i32<@g = external global [8 x i8]> 0
```

Since 4154ada1d485 (#136086, "[Xtensa] Implement Xtensa Floating Point
Option"), `LowerGlobalAddress` returns a generic `load` of the literal
instead of the bare `PCREL_WRAPPER`, and the pattern
`(Xtensa_pcrel_wrapper tconstpool) -> L32R` is gone; `L32R` now matches
`(load (Xtensa_pcrel_wrapper tconstpool))` only. DAGCombine
(`ReduceLoadWidth`) is free to narrow that load, and nothing selects the
wrapper as an address. Built at 8f01edfa112f (the parent of 4154ada1d485),
`llc` compiles all six functions correctly; built at 4154ada1d485, it
crashes.

cc @andreisfr

### Related

- #214974 (open) fixes the same "Cannot select" for a select of FP constants
  (`convertSelectOfFPConstantsToLoadOffset`) by opting out of that
  combine. Its description notes that "any other path needing a constant
  pool *address* would fail the same way". Load narrowing is such a path,
  and #214974 does not cover it (there is no select here).
- Restoring the removed pattern is not a fix. Espressif's fork did that
  (espressif/llvm-project 39a59933237b, `AddedComplexity = -1`), and it
  then compiles `low_byte` to `l32r a8, .LCPI0_0` / `l8ui a2, a8, 0`:
  `L32R` yields the literal's contents, so the narrowed load reads `g[0]`
  instead of the address's low byte. Run under qemu-xtensa, that build
  returns `g[0]`, where GCC's build returns the low byte of `&g`.

Possible fixes: override `shouldReduceLoadWidth` to refuse loads whose
base is a `PCREL_WRAPPER` (this covers the offset cases, whose narrowed
loads read at literal+1..3); or give literal-pool loads their own node so
generic combines leave them alone; or make `LowerConstantPool` materialize
an address, as #214974 suggests for the general fix.
