# Face ID on the board: speed, findings, room for improvement

What the M5Stack CoreS3 (ESP32-S3, 240 MHz, 8 MB PSRAM, 4 MB flash)
taught while the two networks were made fast, and what is left to gain.
Every number here was measured on the board unless it is called an
estimate. Up to the integer lane pipeline, a smoke-test firmware ran
each network on a test vector from the computer and compared the
outputs bit for bit; that firmware is no longer in the repository, and
the last section says how to build one again. From `faceid-10` on, the
numbers come from the application's own log: its `self-test:`,
`profile:` and `cycle:` lines.

## Where it stands

| | First run | Smoke test | `faceid-12` | `faceid-13` |
| --- | --- | --- | --- | --- |
| Recognizer (EdgeFace-XXS), one face | 8,842 ms | 729 ms | 472 ms | **388 ms** |
| Detector (YuNet, 96x64), one frame | 1,568 ms | 330 ms | 97 ms | **99 ms** |
| Outputs against the computer | bit-identical | bit-identical | bit-identical | bit-identical |
| LFW accuracy, integer recognizer | 99.37 % | 99.42 % | 99.42 % | 99.42 % |

The networks alone on CPU0: the first two columns from the smoke test,
the others from the self-test when the application starts.
The recognizer runs on the integer lane pipeline (`nn::lanes`), and
since `faceid-12` the detector's backbone and neck do too (its heads
stay on the earlier kernels of `nn::quant`). Both networks got faster
alone because the application compiles them once
(`Model::compile`): no lookups by name and no plans in a pass. For the
recognizer that was 257 ms, where the estimate was 110. `faceid-13`
made the recognizer's kernels faster, with the same numbers.

In the application they take longer, because the camera and the screen
share CPU0 with them. Measured on 2026-09-28 (details further down):

| In the application | `faceid-10` | `faceid-11` | `faceid-12` | `faceid-13` |
| --- | --- | --- | --- | --- |
| A cycle without a face | 505 ms, of which the detector 485 ms | 430 ms, of which the detector 406 ms | 170 ms, of which the detector 147 ms | **173 ms**, of which the detector 151 ms |
| A cycle that recognizes | 1,710 to 1,750 ms, of which the recognizer 1,095 ms | 1,395 to 1,456 ms, of which the recognizer 904 ms | 929 to 983 ms, of which the recognizer 712 ms | **850 to 903 ms**, of which the recognizer 621 ms |
| From a known face to its name | one cycle | one cycle | one cycle | one cycle |
| Dropped camera frames | 256 in 81 s, with stalls of 100 ms: a third of all time | 6 in 52 s, no stalls | 4 in 30 s, no stalls | 2 in about 35 s, no stalls |
| The stream task's share of CPU0 | not measured | 35 percent | 23 percent | 24 percent |

Against `faceid-10`, the first build measured in the application, a
cycle without a face is three times as fast and a cycle that recognizes
twice as fast (862 ms against 1,730, medians). `faceid-9`, the build
before the speed work, was not measured in the application; it needed
five embeddings for a name where `faceid-10` to `faceid-13` need one.

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

## The work around the kernels (builds `faceid-10` and `faceid-11`)

The kernels above are a part of what a cycle of the application costs.
The rest was found by reading the code and the compiled firmware
(`xtensa-esp32s3-elf-objdump`), not by measuring, so the times in the
first table are estimates from the cost model above. The `cycle:` line
of the log has one number per step to check them with; what the board
said is further down.

What was found:

| Where the time went | Estimate | Why |
| --- | --- | --- |
| Finding tensors by name, recognizer | 70 to 85 ms per face | `Blob::get` walks the table of contents from its start and decodes every entry on the way, with a UTF-8 check of its name: 277 lookups decode 41,107 entries, at 400 to 500 cycles each. The table (40 KB) is in PSRAM. |
| The same, detector | about 45 ms per frame | about 250 lookups, 25,000 entries |
| Making the plans, recognizer | about 33 ms per face | 1,004 group plans per pass, about forty `f32` divisions each, and a division is a software routine |
| The recognizer's input | about 28 ms per face | one division per value, 37,632 values |
| Scaling the frame down | about 16 ms, twice per face | a plain loop with a bounds check and a division per pixel |
| The copy of the frame for the main task | about 11 ms per cycle | 150 KB from PSRAM to PSRAM |
| The panel | about 10 ms per cycle | the timings change with every cycle, so the whole panel was drawn again every cycle |
| The stream task | a third of CPU0, perhaps | it copied every frame of the sensor into PSRAM (about 9 ms each), and held CPU0 for the 18 ms of every preview's SPI transfer |
| The copy of the screen for the live feed | about 6 ms per preview | 88 KB into PSRAM, whether somebody watches or not |
| The decision | 5 embeddings until a name | three for the first average, then three decisions that agree |

One check of the model behind these estimates: PSRAM at 40 MB/s is 6
cycles per byte read, and 12 per byte written (the cache loads a line
before it writes to it). With that, the products, the epilogues, the
GELU lookups and the calls, the stage-0 `fc1` comes to 23.4 ms (measured
23.6) and `fc2` to 14.1 ms (measured 14.5). About 40 percent of a
stage-0 block is PSRAM traffic of its tensors. That part is still there:
it needs the kernels changed (room for improvement, below).

What `faceid-10` does about it, with the networks' outputs unchanged bit
for bit (`crates/vision/tests/fingerprints.rs` pins them):

| Change | Where |
| --- | --- |
| Both networks are compiled when the app starts: every tensor found, every plan made, once | `edgeface::int8::Model`, `yunet::int8::Model` |
| The recognizer's input comes from a table of 256 values | `align::recognizer_input_i8` |
| The frame is scaled down with two table lookups and one addition per pixel | `image::downscale_to_rgb` |
| The warp and the sharpness work on the images' bytes directly; the sharpness sums a row in 32 bits | `warp::warp`, `quality::laplacian_variance` |
| The main task gets the frame's buffer in exchange for its own | `Frame::take` |
| The camera copies only the frames that are wanted | `Camera::capture_on_demand` |
| The stream task sleeps while a preview is on the bus | `Surface::render_from_async` |
| The screen is copied for the live feed only while somebody watches | `logging::mirror_only_when_watched` |
| The panel draws the lines that changed, the timings once per second | `face_id.rs` |
| A sure face is named on its first embedding | `gallery::Decider`; the limits are in the [README](README.md) |

Two settings of the build that may be worth a few percent were left
alone, because their effect can only be measured on the board. Both can
be tried without a change to a file:

```bash
# Optimization across the crates at link time, and one unit of code generation.
CARGO_PROFILE_RELEASE_LTO=thin CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1 cargo dist --bin face_id
# Data cache lines of 64 bytes instead of 32: fewer, longer reads of PSRAM.
ESP_HAL_CONFIG_DATA_CACHE_LINE_SIZE=64B cargo dist --bin face_id
```

`lto = "fat"` does not build: the Xtensa code generator of the compiler
stops with "Cannot select: XtensaISD::PCREL_WRAPPER" (toolchain `esp`,
September 2026). The caches themselves are already as large as the chip
allows (32 KB for instructions, 64 KB for data): that is esp-hal's
default.

### What `faceid-10` measured

One log of 81 seconds on the board, with the autoflash page open (so the
live screen feed was watched):

| Step | Cycles without a face | Cycles that recognize |
| --- | --- | --- |
| The whole cycle | 500 to 520 ms | 1,711 to 1,751 ms |
| Waiting for the frame | 1 to 3 ms | 2 to 31 ms |
| Scaling the frame down (`scale`) | 18 to 25 ms | 18 to 51 ms |
| The detector with the decoding (`detect`) | 457 to 504 ms | 458 to 498 ms |
| Cutting the face out (`align`) | | 63 to 93 ms |
| The recognizer (`embed`) | | 1,093 to 1,107 ms |
| The decision (`decide`) | | 15 to 43 ms |
| The preview | 9.6 to 10 frames per second | 8.4 to 10.3 |

- **A known face was named in its first recognizing cycle**, at scores
  of 0.92 to 0.97 (the sure limit of one frame is 0.55).
- **The networks are half as slow again in the application** as alone:
  the detector 485 ms against 330, the recognizer 1,095 against 729. The
  first cycle after the start had the detector at 362 ms. What CPU0 does
  besides was not measured in `faceid-10`; `faceid-11` reports it
  (`stream` in the `cycle:` line).
- **Cutting the face out took 90 ms**, against an estimate of 20 to 30.
  `faceid-11` logs its four steps one by one.
- **256 camera frames were dropped** (the ring buffer of 40 rows
  overflowed), three per second on average, in bursts. 12 of the 59
  cycles without an embedding took 665 ms to 4.4 s instead of 505, and
  recognizing cycles up to 6.3 s. Above the typical time of each step,
  the cycles lost 25 of 72 seconds (the cycle that saved to flash left
  out): about 96 ms per dropped frame.
- **The IMU on CPU1 lost samples at the same moments** ("sample gap":
  no sample for 50 ms). CPU1's timers are served by the timer interrupt
  on CPU0, at priority 1.

The cause of the last two is in the stream task, which runs as an
interrupt handler at priority 1 on CPU0. After an overflow, the
camera's `begin_frame` started the capture again and **waited** for the
next frame boundary and a whole frame: 50 to 100 ms spinning inside the
interrupt handler. Meanwhile the main task stood still, and so did the
timer interrupt of both cores, which has the same priority. Why the
first overflow of a burst happens is not known yet: `faceid-11` logs the
longest time between two times the ring was emptied (`longest pump gap`;
the ring holds about 5 ms).

### What `faceid-11` changes

| Change | Where |
| --- | --- |
| The stream task never waits while it has CPU0: the camera starts again without waiting for the sensor, and the panel sleeps while its pixels are on the bus | `Camera::service`, `advance`, `current`; `Canvas::show_async` |
| Only every fourth preview goes to the live screen feed, which shows two to three camera frames per second anyway | `Surface::without_mirror` |
| Only the part of the half-size frame that the alignment reads is scaled down: on the fixture photo, 72 percent of it | `image::downscale_to_rgb_within`, `align::source_region` |
| The impostor bank is copied into PSRAM, which reads four times as fast as flash | `face_id.rs` |
| The `cycle:` line reports the stream task's time on CPU0, the frames dropped and the longest pump gap; an `align:` line reports the steps of cutting the face out | `face_id.rs` |

The networks' outputs are still the same bit for bit: the fingerprint
test checks the partial scaling against the full one, and a property
test checks that the warp reads nothing outside `warp::footprint`.

### What `faceid-11` measured

One log of 52 seconds, with the autoflash page open, 62 cycles without
an embedding and 18 with one (medians):

| Step | Without an embedding | With one |
| --- | --- | --- |
| The whole cycle | 430 ms | 1,426 ms |
| `detect` | 406 ms | 411 ms |
| `align` | | 68 ms |
| `embed` | | 904 ms |
| `decide` | | 15 ms |
| `stream` | 153 ms (36 percent) | 498 ms (35 percent) |

- **The stalls are gone.** 6 frames dropped in 52 seconds, one or two
  at a time, and no cycle was slowed down by them. The longest pump gap
  was 10 ms, the median 4 ms. The IMU on CPU1 lost no sample.
- **The stream task takes 35 percent of CPU0**, 353 ms per second, or
  36 ms for each of its 9.7 previews per second. If that share is
  spread evenly over the steps, the networks' own time is about 262 ms
  for the detector and 591 ms for the recognizer, 20 percent less than
  alone before the speed work (330 and 729 ms). The compiled models (no
  lookups by name, no plans per pass) were estimated to save 14 and
  15 percent; the self-test of `faceid-12` measures the networks alone.
- **One reason for the 36 ms**, found in the code after the log: after
  a preview, the stream task asked for a new frame while the frame it had
  asked for before the preview was still arriving, so the camera copied
  two frames per preview into PSRAM and one was never drawn.
- **Cutting the face out took 47 to 91 ms**: scaling down 11 to 36 ms,
  the warp 25 to 63 ms, the sharpness 5 to 12 ms, the recognizer's input
  3 to 11 ms. The warp is the largest part; all four vary with the
  stream task's work at the same moment.

### What `faceid-12` changes

| Change | Where |
| --- | --- |
| The detector's backbone and neck run on the lane kernels: the first convolution, the 1x1 and depthwise convolutions of every unit, and the max pooling. The plans are made when the model is compiled | `yunet::int8::Model` |
| The camera copies one frame per request, however often the stream task asks while it is on its way | `Camera::request_frame` |
| The application checks both networks on the board against the computer when it starts, before the camera does, and logs their times alone and the lane kernels' scalar fallbacks | `nn::check`, `face_id.rs`, `lanes::fallbacks` |
| The `cycle:` line reports the frames copied and the time the copies took | `Camera::take_stats` |

The detector's numbers change with the lane kernels, as agreed: their
epilogue rounds in two steps where the old kernels rounded once. On the
64 calibration frames (`facekit quantize yunet`, which leaves the
weights file unchanged) every stage stays within 0.2 dB of the old
kernels, and the detections are the same: all 64 agree with the `f32`
detector, worst centre 2.17 px (2.18 before), size 3.20 px, landmark
3.15 px. On the fixture face the box and the landmarks are within
0.19 px of the `f32` detector's. The recognizer, the impostor bank and
the thresholds do not depend on the integer detector: `facekit`
measures them with its own detector on the computer.

### What `faceid-12` measured

The self-test at the start:

```text
self-test: detector 97 ms, recognizer 472 ms alone, 0 scalar fallbacks; both compute what the computer computes
```

One log of 30 seconds, with the autoflash page open, 120 cycles without
an embedding and 10 with one (medians):

| Step | Without an embedding | With one |
| --- | --- | --- |
| The whole cycle | 170 ms | 950 ms |
| `detect` | 147 ms | 139 ms |
| `align` | | 58 ms |
| `embed` | | 712 ms |
| `decide` | | 13 ms |
| `stream` | 40 ms (23 percent) | 219 ms (23 percent) |

- **The detector is 3.4 times as fast alone** (97 ms against 330) and
  2.8 times in the application (147 ms against 406).
- **The stream task's share fell from 35 to 23 percent**: one copy per
  frame used. Per second it copies 10 frames at 9.8 ms each (99 ms) and
  draws 9.3 previews at 14.4 ms each besides.
- **In the application each network takes about 1.5 times its time
  alone** (147 against 97 ms, 712 against 472). The stream task's 23
  percent explains 1.3; the rest is most likely the cache: both cores
  share it, and the stream task moves a frame of 150 KB through it ten
  times per second.
- **4 frames dropped in 30 seconds**; the longest pump gap was 9 ms.

The self-test runs each network once on made-up inputs (`check::noise`)
and compares a fingerprint of the outputs with `check::DETECTOR` and
`check::RECOGNIZER`, which `tests/fingerprints.rs` computes on the
computer. It replaces the smoke test's check of the whole networks; it
does not check each assembly primitive on its own, as the smoke test
did. A log line of `faceid-12` looks like this, with the times filled
in:

```text
self-test: detector <ms> ms, recognizer <ms> ms alone, 0 scalar fallbacks; both compute what the computer computes
```

### What `faceid-13` changes

Only the recognizer's kernels, and none of its numbers: the fingerprints
of `tests/fingerprints.rs` and `check::RECOGNIZER` are those of
`faceid-12`, and the self-test checks the board against them.

| Change | Where |
| --- | --- |
| LayerNorm sums a row and its squares on the vector unit (two `ee.vmulas.s16.accx` loops) and derives the squares about the mean from them in integers; the mean's division runs in 32 bits. Before: two scalar `i64` loops of about 30 instructions per value, and a 64-bit division and conversion per row | `lanes::layer_norm`, `model::norm_constants` |
| Fewer assembly calls: the batch of groups per call went from 8 to 128, so a row of a layer takes one call per 12 KB of its weights (the part that stays in the cache): the stage-0 `fc1` one instead of two, the stage-2 `fc1` three instead of six; a convolution one per pixel | `lanes::GROUP_BATCH` |
| The stem's and stages 0 and 1's linear and convolution weights are also kept as `i16` (123 KB in PSRAM): their inner loop is 6 instructions per two inputs instead of 10, without the widening from 8 bits | `LaneWeight::wide`, `edgeface::int8::MODEL_WIDE_LEN` |
| The MLP's hidden tensor is written and read in strips of rows in 36 KB of internal RAM, not whole in PSRAM: five strips in stage 0, three in stage 1, one in stages 2 and 3 | `Scratch::with_hidden`, `HIDDEN_STRIP_LEN` |
| The GELU table holds only the inputs whose rounded GELU is neither 0 nor the input itself (-3,732 to 3,732 in units of 2^-10): 15 KB of internal RAM instead of 64, the same values for every input (`tests/int8_lanes.rs` checks all 65,536) | `GeluTable` |
| After its timed pass, the self-test runs the recognizer once more with a trace and logs the time of each part | `profile_recognizer` in `face_id.rs` |

Internal RAM: the table shrinks by 49 KB, the strip takes 36 KB.

What it should give, estimated from the cost model and not measured: the
recognizer alone from 472 ms to about 380. The strips save the most
(about 40 ms: 340,000 values of the hidden tensor per face, each written
to PSRAM and read back at about 36 cycles), then LayerNorm (about
25 ms), the 16-bit weights (about 20 ms on 19 million products) and the
fewer calls (about 5 ms).

The profile lines, after the `self-test:` line:

```text
profile: recognizer <ms> ms traced; stem <ms> ms, head <ms> ms
profile: stage 0 <ms> ms: downsample 0.0, conv blocks <ms> + <ms>, attention 0.0, mlp 0.0
profile: stage 1 <ms> ms: downsample <ms>, conv blocks <ms>, attention <ms>, mlp <ms>
...
```

Each part is the time between two trace calls, including the copy of
its output to `f32` for the trace, so the traced pass takes a little
longer than the timed one. `attention` is the SplitTransposeBlock up to
the attention's projection: the split convolutions, the positional
encoding and the attention itself.

### What `faceid-13` measured

The start of the log:

```text
MEM [face id ready] internal=108/144 KiB (peak 108 KiB) | psram=3735/8192 KiB (peak 3735 KiB)
profile: recognizer 449.8 ms traced; stem 25.4 ms, head 4.5 ms
profile: stage 0 85.2 ms: downsample 0.0, conv blocks 41.9 + 43.2, attention 0.0, mlp 0.0
profile: stage 1 103.7 ms: downsample 13.4, conv blocks 27.7, attention 38.5, mlp 23.9
profile: stage 2 151.4 ms: downsample 8.0, conv blocks 20.6 + 20.3 + 20.7 + 20.9 + 20.2, attention 21.5, mlp 19.0
profile: stage 3 79.3 ms: downsample 9.4, conv blocks 18.9, attention 32.4, mlp 18.4
self-test: detector 99 ms, recognizer 388 ms alone, 0 scalar fallbacks; both compute what the computer computes
```

One log of about 35 seconds: an enrollment of six samples, then two
recognitions; 113 cycles without an embedding and 17 with one (medians):

| Step | Without an embedding | With one |
| --- | --- | --- |
| The whole cycle | 173 ms | 862 ms |
| `detect` | 151 ms | 152 ms |
| `align` | | 58 ms |
| `embed` | | 621 ms |
| `decide` | | 14 ms |
| `stream` | 40 ms (23 percent) | 207 ms (24 percent) |

- **The recognizer alone went from 472 to 388 ms** (18 percent; the
  estimate was about 380), bit for bit the same. In the application it
  went from 712 to 621 ms, and a cycle that recognizes from 950 to 862.
  With that, both kinds of cycle are at least twice as fast as in
  `faceid-10`.
- **Internal RAM** holds 108 KB, as estimated: the table's 49 KB fewer,
  the strip's 36 KB more.
- **2 frames dropped in about 35 seconds**, one of them at the start and
  one while the enrollment was saved to flash (which parks CPU1); the
  longest pump gap was 8 ms.
- **In the application the recognizer takes 1.6 times its time alone**
  (621 against 388 ms; 1.5 in `faceid-12`). The part of it that the
  kernels cannot remove, the stream task and the shared cache, is now a
  larger share.
- **The traced pass is 62 ms longer than the timed one**: the copies to
  `f32` for the trace, one per part, mostly in stages 0 and 1 where the
  tensors are large. The profile's parts are therefore a few
  milliseconds too long each.
- **Where the time goes**: stage 2 is the largest (151 ms traced, five
  ConvBlocks of about 20 ms), then stage 1 (104), stage 0 (85) and stage
  3 (79). The three attentions take 92 ms together.
- **Stage 3's attention is out of proportion**: 32 ms for 9 tokens,
  more than stage 2's (21.5 ms for 49 tokens). Its products are few, but
  it has 42 x 42 channel pairs per head, 7,056 in all, and per pair
  `f32` work in library routines: the conversion of each integer dot
  product to `f32` (`__floatdisf`), `expf`, a division in the softmax
  and `roundf` (read from the disassembly, not timed). See item 10 of
  the next section.

### What `faceid-14` changes

No speed work: the fixes of a review, with the same numbers (the
fingerprints are unchanged).

- Once the face has been gone for a second, the vote forgets also the
  decisions that had not shown anything yet. Before, two plain decisions
  for a person could remain, and the next face was then named after one
  more.
- `cargo dist` no longer pads the image to the size of the flash, so
  flashing through autoflash keeps the enrollments.
- The live screen feed counts a page that stays connected while the
  board restarts as a viewer, so the feed does not freeze.
- A requested camera frame that is lost (an overflow, a wrong length)
  is replaced by the next one, and the drawing that never waits empties
  the camera's buffer at least once per batch of rows.
- The terminal's messages fit its 19 columns, and the panel rounds the
  score and the limit (it cut off the last digit), and the limit moves
  in whole hundredths.

### What `faceid-14` measured

- **The enrollments survived the new firmware**: `flash: 1 people
  loaded`, the person enrolled under `faceid-13`, flashed over with the
  image of `--skip-padding`.
- **The self-test**: detector 98 ms, recognizer 383 ms alone, 0 scalar
  fallbacks, the same numbers as the computer; so the reworked operand
  checks of the assembly work on the board.
- **The same speed as `faceid-13`** (medians of 69 cycles without an
  embedding and 22 with one): 170 ms and 855 ms, `embed` 610 ms.
- **No dropped frame** in about 35 seconds; the longest pump gap was
  4 ms.
- **Three visits of the enrolled person**, each named after its first
  frame (score 0.83 to 0.89), and `face gone` a second after each left.

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

In the kernels, in order of what they are worth.

1. **The detector on the lane kernels** (330 ms, perhaps 60 ms): done
   in `faceid-12` for the backbone and the neck, 97 ms alone. The heads
   (1, 4 and 10 filters, which would need padding to eight) and the
   upsample-and-add stay on the earlier kernels.
2. **Rows inside the assembly** for the narrow layers. A 24-wide row of
   the stage-0 `fc1` is twelve groups, each call with its operand checks
   in Rust: about 50 cycles per output against 25 in the loop itself.
   Since `faceid-13` that row is one call (it was two), but still one
   call per row; the row loop inside the assembly would save the rest.
3. **LayerNorm's row constants in the vector unit**: done in
   `faceid-13`, the sums of a row and of its squares through `ACCX`. Per
   row there remain two `f32` divisions, a square root and a rounding,
   all library routines.
4. **GELU without a table.** The lookup is ten cycles per value on
   340,000 values per face; a piecewise polynomial in the vector
   registers would be one or two. Since `faceid-13` the table is 15 KB.
5. **Weights as `i16` where they are small**: done in `faceid-13` for
   the stem and stages 0 and 1. Not for stages 2 and 3, whose cost is
   reading the weights.
6. **The stem and the detector's first convolution** widen a 3-channel
   image to eight 16-bit channels in scalar code (8 ms for the
   detector); the camera frame could be converted to that form directly.
7. **The second core.** CPU1 runs the capability tasks and is mostly
   idle. Half of each layer's rows on CPU1 would come close to halving
   the time, at the price of a change to the framework.

The detector reached 97 ms with item 1; `faceid-13` does items 3 and 5
and part of 2, 4 and 8 for the recognizer.

Two more, from the estimates of the section on the work around the
kernels:

8. **Blocks in strips.** A block's tensors go through PSRAM once per
   layer: about 40 percent of a stage-0 block. One image row at a time
   through the depthwise convolution, the LayerNorm and both layers of
   the MLP, with the scratch in internal RAM, leaves only the stream in
   PSRAM, and the 150 KB hidden tensor never exists. The stream then
   needs two buffers, because the depthwise convolution reads the old
   values of the rows next to its own. It fits with item 2. Since
   `faceid-13` the MLP's hidden tensor goes through internal RAM in
   strips; the other tensors of a block still go through PSRAM.
9. **Loops without a branch.** The inner loop of the products spends 4 of
   its 14 cycles on the counter and the branch. The chip has loop
   instructions (`loopnez`) that cost nothing per round.
10. **The attention's work per channel pair** (found by the profile of
    `faceid-13`). Per pair of channels of a head, the attention converts
    a 64-bit dot product to `f32` in a library routine, multiplies four
    times, takes `expf`, divides by the row's sum and rounds with
    `roundf`: stage 3 has 7,056 pairs and its attention takes 32 ms for
    9 tokens. Without changing the numbers: the 32-bit conversion where
    the sum fits (`quant::sum_to_f32`, as LayerNorm does). With new
    numbers, to be checked with `facekit` on LFW: the reciprocal of the
    sum instead of a division per value, and an `expf` of lower
    precision.

## Building a smoke test again

The check that made this work safe is small, and worth rebuilding before
any change to the kernels. The application's self-test already does the
first two parts on made-up inputs: `nn::check` compares a fingerprint of
each network's output with the computer's, and `profile_recognizer` in
`face_id.rs` times each block. The third part is what it lacks.

- on the computer, run both integer networks on fixed inputs
  (`Model::compile` once, then `Model::forward`, in `edgeface::int8` and
  `yunet::int8`, with the fixtures of `crates/vision/tests/fixtures/`)
  and write the inputs and outputs to an FKB1 file;
- on the board, run the same inputs and compare the outputs bit for bit;
  time each block through `Model::forward_traced`, whose callback fires
  after every block;
- before the networks, run each assembly primitive on random data and
  compare it with `lanes::model` (`model::groups`,
  `model::depthwise_pixel`, `model::norm_apply`, `model::dot`,
  `model::mix`, `model::add`, `model::rescale`, `model::max4`): a wrong
  instruction shows as one named primitive, not as a wrong face;
- give every flashed image an identifier in its first log line.
