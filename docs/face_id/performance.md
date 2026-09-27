# Face ID on the board: speed, findings, room for improvement

What the M5Stack CoreS3 (ESP32-S3, 240 MHz, 8 MB PSRAM, 4 MB flash)
taught while the two networks were made fast, and what is left to gain.
Every number here was measured on the board with a smoke-test firmware
that ran each network on a test vector from the computer and compared
the outputs bit for bit. That firmware is no longer in the repository;
the last section says how to build one again.

## Where it stands

| | First run | Now |
| --- | --- | --- |
| Recognizer (EdgeFace-XXS), one face | 8,842 ms | **729 ms** |
| Detector (YuNet, 96x64), one frame | 1,568 ms | **330 ms** |
| Outputs against the computer | bit-identical | bit-identical |
| LFW accuracy, integer recognizer | 99.37 % | 99.42 % |

The recognizer runs on the integer lane pipeline (`nn::lanes`). The
detector still runs on the earlier kernels (`nn::quant`): it is the
largest remaining gain.

## The board's cost model

Measured, because estimates from the source were two to four times too
low every time.

| What | Cost |
| --- | --- |
| Register instructions, code in flash | 1.07 cycles each (1.01 from internal RAM) |
| The same, code larger than the 16 KB instruction cache | 2.4 cycles each |
| Loops with loads and stores | about 1.5 cycles per instruction |
| A taken branch | about 3 cycles (no branch prediction) |
| A vector multiply-accumulate (8 products) | 1 cycle |
| Reading a register of the accumulator after accumulating | about 10 cycles |
| `f64` arithmetic, `log2`, `exp2`, `sqrt` | software routines, hundreds of cycles |
| `f32` division, `i64` to `f32` | library routines |
| Flash (DIO, 40 MHz) | about 10 MB/s |
| PSRAM (quad, 80 MHz) | about 40 MB/s |
| Internal RAM | not cached, a few cycles per access |

Two consequences shaped everything:

- **Per-value scalar code between integer layers costs 60 to 100 cycles
  per value**, however carefully it is written: a store with scale and
  bias, a quantize, a table lookup each compile to 15 to 40 instructions
  with loads, stores and branches. The products themselves cost 0.7
  cycles each on the vector unit. A network whose values leave the
  vector registers between layers spends nine tenths of its time there.
- **The caches are not the problem.** The instruction and data caches
  are shared with CPU1, which runs the radio, audio and sensor tasks,
  but code from flash runs as fast as code from RAM as long as the hot
  loop fits the cache. What costs is bandwidth: a tensor of 150 KB read
  and written through PSRAM is 7 ms, and the 1.4 MB of weights read once
  per face are 35 ms from PSRAM and 140 ms from flash.

## What was built, in order

| Step | Recognizer | Detector | What changed |
| --- | --- | --- | --- |
| First run | 8,842 ms | 1,568 ms | scalar kernels, `i64` sums |
| `i32` partial sums | 5,113 | 973 | an `i64` multiply is a library call |
| Blocked scalar kernels | 3,724 | 585 | four outputs per pass over the input |
| Vector unit | 2,333 | 531 | dot products through `ACCX`, depthwise sums through `QACC` |
| Work around the loops | 1,560 | 448 | reciprocals instead of divisions, inline rounding, weights chunked to stay in the cache |
| Compiled-code fixes | 1,418 | 425 | calls per output removed (`__floatdisf`, a store that was not inlined, a `memset` per batch) |
| Stores per run, packed weights | 1,006 | 329 | one output choice per run of channels, eight outputs per accumulator lane, word-wise accumulator decode |
| Integer lane pipeline | **729** | 330 | recognizer only: every tensor `i16`, requantization, residual add and LayerNorm in the vector registers |

### The integer lane pipeline (`nn::lanes`)

Every tensor between layers is `i16` with one symmetric mapping, and a
layer runs from its input to its output in the vector registers:

1. the eight 40-bit accumulator lanes start from a *bias image* (the
   bias in whole product units plus half a unit of the shift that
   follows), loaded with `ee.ld.qacc`;
2. the products accumulate: one 16-bit input value is broadcast to the
   eight lanes and multiplied by the weights of eight filters at that
   input, which are stored together (`nn::pack`) and widened from 8 to
   16 bits in the registers;
3. `ee.srcmb.s16.qacc` shifts and saturates the sums to 16 bits;
4. each lane is multiplied by its own factor through the accumulator and
   shifted again, with a per-lane half for rounding and a per-lane
   offset for the part of the bias below one product unit;
5. the result is stored, added to the residual stream, clamped for a
   ReLU, or clamped and looked up in the GELU table while the eight
   values are in the cache.

LayerNorm runs through the accumulator too (sums with `ACCX`, one
inverse square root per pixel, then a per-lane scale and offset); the
attention's covariance is exact integer dot products and its mixing an
integer weighted sum, with only the softmax in `f32`. The computer runs
the identical integer arithmetic in scalar code (`lanes::model`).

Stage-0 block (784 pixels, 24 channels, MLP width 96), in milliseconds:

| Step | Before | Lanes |
| --- | --- | --- |
| Stream to the depthwise input | 8.2 (`f32` to `i8`) | none: the convolution reads the stream |
| Depthwise 3x3 | 27 | 5.3 |
| LayerNorm | 19 (three passes) | 7.4 |
| `fc1` with GELU | 47 + 26 + 25 (layer, table, quantize) | 23.6 |
| `fc2` with the residual add | 24 + pass | 14.5 |
| The block | 166 | 70 |

## What went wrong, and how it was found

Worth keeping, because each cost at least one build.

- **Estimating instead of measuring.** Instruction counts from the
  source were wrong by a factor of two to four. The compiled kernels
  called `__floatdisf` per output (the compiler had hoisted the 64-bit
  conversion above its range check), called a store it had been asked to
  inline, cleared 512 bytes per batch, and decoded the accumulator one
  byte at a time (200 instructions for 40 bytes). Reading the
  disassembly of the hot loop, and timing each primitive alone on the
  board, found every one of them.
- **`f64` in per-layer and per-row constants.** The plans of the lane
  kernels and the LayerNorm's row constants used `f64` with `log2`,
  `exp2` and `sqrt`: 32 ms per stage-0 LayerNorm, 50 ms per stage-3
  block. `f32`, with the exponent read from the float's bits, removed
  it.
- **Overlapping accumulator stores.** Two store instructions to the same
  16-byte line in a row left stray bytes; each of the four stores now has
  its own slot.
- **An accumulator image packed in 128 bits.** Four lanes of 40 bits are
  160 bits; lane 3 lost its top bits. Found by testing each kernel alone
  against the `f32` reference on the computer.
- **A saturation one step apart** between the assembly (+16384) and the
  scalar model (16383) at the GELU clamp. Found by the board's check of
  every primitive against the model.
- **Layer scale folded before the shift.** See the accuracy notes in the
  [README](README.md): 20 dB in stage 2.
- **A lookup table in PSRAM.** The GELU table's reads are random, and at
  64 KB half of them missed the cache: 35 cycles per lookup. It lives in
  internal RAM now.
- **Copying the weights in one go.** A continuous 1.4 MB copy from flash
  starves CPU1's audio and sensor tasks of the flash they execute from.
  The application copies one tensor at a time with pauses.
- **Parking CPU1 while it holds a lock.** See "Enrollments in flash" in
  the [README](README.md).
- **QIO flash mode bricks the board**, and the PSRAM runs at 80 MHz only
  with the timing the board module sets; both were settled early and
  should not be revisited without a recovery plan.

## Room for improvement

In order of what they are worth.

1. **The detector on the lane kernels** (330 ms, perhaps 60 ms). Its
   activations are already 16-bit with per-tensor mappings, so the same
   kernels apply; what it pays today is the scalar store per output (its
   first two stages alone are 120 ms for 50,000 outputs). Its head
   convolutions have 1, 4 and 10 filters, which need padding to eight.
   The max pooling and the upsample-and-add have lane kernels already
   (`lanes::max_pool_2x2`, `lanes::add`, `lanes::rescale`).
2. **Rows inside the assembly** for the narrow layers. A 24-wide row of
   the stage-0 `fc1` is twelve groups in two assembly calls, each with
   its operand checks in Rust: about 50 cycles per output against 25 in
   the loop itself. One call per layer with the row loop inside would
   save perhaps 15 ms per stage-0 block.
3. **LayerNorm's row constants in the vector unit.** The sums of a row
   and of its squares are scalar `i64` loops today (7.4 ms per stage-0
   block); `ee.vmulas.s16.accx` does both in a few instructions per
   eight values.
4. **GELU without a table.** The lookup is ten cycles per value on
   340,000 values per face; a piecewise polynomial in the vector
   registers would be one or two.
5. **Weights as `i16` where they are small.** Widening 8-bit weights in
   the registers is three of the four instructions per eight products.
   The stage-0 and stage-1 layers have few weights and many rows; stored
   as 16-bit they would halve their loops. Not for stages 2 and 3, whose
   cost is reading the weights.
6. **The stem and the detector's first convolution** widen a 3-channel
   image to eight 16-bit channels in scalar code (8 ms for the
   detector); the camera frame could be converted to that form directly.
7. **The second core.** CPU1 runs the capability tasks and is mostly
   idle. Half of each layer's rows on CPU1 would come close to halving
   the time, at the price of a change to the framework.

With 1 to 4 the recognizer should reach about 450 ms and the detector
about 60 ms.

## Building a smoke test again

The check that made this work safe is small, and worth rebuilding before
any change to the kernels:

- on the computer, run both integer networks on fixed inputs
  (`edgeface::int8::forward`, `yunet::int8::forward` with the fixtures of
  `crates/vision/tests/fixtures/`) and write the inputs and outputs to an
  FKB1 file;
- on the board, run the same inputs and compare the outputs bit for bit;
  time each block through `forward_traced`, whose callback fires after
  every block;
- before the networks, run each assembly primitive on random data and
  compare it with `lanes::model` (`model::groups`,
  `model::depthwise_pixel`, `model::norm_apply`, `model::dot`,
  `model::mix`, `model::add`, `model::rescale`, `model::max4`): a wrong
  instruction shows as one named primitive, not as a wrong face;
- give every flashed image an identifier in its first log line.
