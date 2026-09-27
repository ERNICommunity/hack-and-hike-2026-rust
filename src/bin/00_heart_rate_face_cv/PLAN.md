# Plan: heart rate from the face (camera + face detection + green signal)

Goal: the camera image on the left half of the screen. The user fills it
with their face (a box around a detected face comes back in the last
steps). The right half is a dashboard: the FPS, the green signal of the last
20 s, and the spectrum of the last 10 s with the heart rate (40–180 BPM).
This method is called **rPPG** (remote photoplethysmography). With each heartbeat, blood
fills the skin a little. Blood absorbs green light, so the face gets a tiny
bit darker in the green channel on each beat.

Rules for this project:

- Every file lives in `src/bin/00_heart_rate_face_cv/`. Nothing outside it
  changes: no `Cargo.toml`, no library code, no new crates.
- **One exception:** the camera code gets a way to lock auto exposure and
  auto white balance (section 6). It stays backward compatible: apps that
  do not call the new function work exactly as before.
- Reuse the library (`hack_and_hike::...`) and the crates the project
  already has: `embedded-graphics`, `libm`, `arrayvec`, `embassy-*`, `log`.
- Build in small steps. After each step, the program builds, runs on the
  board and shows something.

## 1. What we reuse

| Need | Existing piece | Where |
| --- | --- | --- |
| Camera frames, 320x240 RGB565 | `Camera::begin_frame`, `Frame::scanline`, `Frame::finish`, `pump` | `src/capabilities/camera/` |
| Stream the camera straight to the panel, no copy | `Surface::render_from` + the `ScanlineSource` trait | `src/capabilities/display/mod.rs` |
| Show only the middle of the camera image | the `CenteredCrop` pattern (we copy its ~15 lines, because it is private to the demo) | `src/bin/demo/screens/camera.rs` |
| Draw text, lines, plots | `embedded-graphics` primitives into our own PSRAM image (not `Canvas`: see "Measured", step 2), `common::DENSE_FONT`, `theme::*` | `src/ui/` |
| Large buffers without using the stack | `psram::leaked_slice`, `psram::leaked_value` | `src/board/psram.rs` |
| sin, cos, sqrt without `std` | `libm` | dependency |
| Time stamps, FPS | `embassy_time::Instant` | dependency |
| Program skeleton | `template.rs` | `src/bin/template.rs` |

## 2. Screen layout (320x240)

```text
x: 0            160                          320
   ┌─────────────┬──────────────────────────┐ y=0
   │             │                 12.3 fps │   ← small, top right
   │   camera    │ green, 20 s   (max)      │
   │  160x240    │  ╱╲╱╲_╱╲╱╲_╱╲             │   ← top half: signal
   │  middle     │               (min)      │
   │  crop,      ├──────────────────────────┤ y≈126
   │  filled     │  HR 72 BPM               │
   │  with the   │      ▁▂█▂▁               │   ← bottom half: spectrum
   │  face       │ 40               180 BPM │     of the last 10 s
   └─────────────┴──────────────────────────┘ y=240
```

- Left: the middle 160 columns of the camera at full height (portrait
  crop, no scaling). This is like the demo's `CenteredCrop`, but narrower.
  No boxes until face detection (the last steps).
- The y axis of the signal plot runs from the min to the max value of the
  window, so the small pulse wave becomes visible.
- "40–180" means beats per minute (BPM), which is 0.67–3 Hz.
- The signal plot shows the last 20 s. The heart rate uses only the last
  10 s, so it follows changes faster and costs less to compute.

## 3. Constraints we must respect (found in the code)

1. **The camera ring buffer overflows within a few milliseconds.** It holds
   40 rows, about 5 ms of sensor data. If the CPU works longer than that
   without calling `pump()`, the frame is dropped and capture restarts
   (`camera/capture.rs`). So every longer computation (face detection,
   drawing the dashboard) must call `camera.pump()` or `frame.pump()` every
   ~2 ms, for example once per image row it processes.
2. **The loop must not sleep.** Same reason. The demo's camera screen
   returns `may_idle() == false` for this. We only use
   `embassy_futures::yield_now().await`. Only our `main` runs on CPU0, so
   this does not waste time.
3. **The SPI bus to the panel is the slow part:** about 31 ms for a full
   screen, so about 15 ms for one half. The camera half is sent every
   frame. The dashboard is redrawn 4 times per second, and the camera is
   pumped while it is drawn and sent.
4. **No big arrays on the stack.** The `large_stack_frames` lint denies them,
   and CI runs `cargo clippy --release -- -D warnings` on every binary. Our
   binary must pass it, or CI fails. Big buffers go to PSRAM.
5. **Auto exposure (AE) and auto white balance (AWB) are on after
   start-up.** When they adjust, the brightness jumps and the signal gets
   steps that are much bigger than the pulse. Step 3 adds
   `Camera::set_auto_adjust` to lock them (section 6).
6. ~~Can a binary name start with a digit?~~ **Yes** (checked in step 0):
   `cargo dist --bin 00_heart_rate_face_cv` builds, and clippy passes.
7. **PSRAM is slow** (measured in step 2): reading one 160x240 crop of a
   frame takes ~16 ms, about 5 MB/s. Every pass over pixels in PSRAM costs
   milliseconds, so it must pump the camera as it goes (for example once
   per row), and big writes must be split (the dashboard clears its image
   in slices of 20 rows).

## 4. Face detection: which model fits the chip?

Hardware: ESP32-S3, 2 cores at 240 MHz, single-precision FPU, about 512 KB
internal RAM plus PSRAM (external, slower). CPU0 runs our code alone. CPU1
is busy with the library's sensor tasks.

The times are **rough estimates to check by measuring**. They are not
measured values.

| Option | Speed (est.) | Quality | Fits our rules? |
| --- | --- | --- | --- |
| **A. Skin colour** (YCbCr thresholds on a small image, box around the largest skin area) | 1–3 ms | low–medium: fails on skin-coloured backgrounds and with bad light | yes, ~60 lines |
| **B. LBP cascade** (OpenCV's `lbpcascade_frontalface_improved`, integer-only, 24x24 window on an 80x120 grey image) | ~10–60 ms | medium: good for frontal faces in normal light, some false hits | yes: a small Python script converts the XML into a Rust table |
| C. Haar cascade (`haarcascade_frontalface_default`) | ~3–5x slower than B | similar to B | yes, but no benefit over B |
| D. Tiny own CNN (PyTorch → **ONNX** → weights as Rust arrays, conv/ReLU/pool written by hand) | ~50–500 ms, depends on size | medium–high if trained well | yes, but needs training data and training |
| E. Espressif ESP-DL `human_face_detect` (made for the S3, uses SIMD) | ~tens of ms | high | **no**: C++/ESP-IDF, would change the whole build |
| F. ONNX Runtime, `tract`, TFLite Micro | – | – | **no**: need `std`/an OS or C++ |
| G. `no_std` Rust inference crates (`microflow`, `burn`) | ? | depends on model | **no** for now: needs a new crate in `Cargo.toml` |

About ONNX: no ONNX *runtime* runs on this `no_std` chip. ONNX is still
useful as a file format to take weights out of PyTorch (option D).

**Our choice:** A first. Then **B as the sweet spot**: proven quality, no
training, integer maths only, and a tiny model (a few KB). D only if B is
not good enough.

**When:** face detection is now the **last** part of the project (steps
5–7). Until then, the whole view is the measurement area, and the user
fills it with their face. So the heart rate works first, and the detector
only makes it easier to use.

The detector does **not** need to run on every frame. The face moves
slowly. It runs every N frames (e.g. every 5th, or every 200 ms), and the
box is smoothed in between. This keeps the FPS and the green sampling rate
high.

## 5. Signal processing (heart rate)

Per frame:

1. **ROI** (region of interest): for now, the **whole 160x240 view**. The
   user fills it with their face. With face detection (steps 5–7), it
   becomes the inner part of the face box: the middle 60% of the width and
   the rows from 10% to 70% of the height (forehead and cheeks, less hair,
   background and mouth movement).
2. **Green mean:** the average of the green channel over the ROI. RGB565
   has 6 bits for green (0–63), the most of the three channels. Averaging
   thousands of pixels gives a precision much finer than 1 step.
3. Store `(time in ms, green mean)` in a ring buffer of the last 20 s
   (≈ 600 samples at 30 FPS; in PSRAM). The top plot shows all of it.

A few times per second (when the dashboard is redrawn), on **only the last
10 s** (`HR_WINDOW`, ≈ 300 samples at 30 FPS):

4. **Remove the trend:** subtract the best straight line through the 10 s
   (a least-squares line fit). This removes the mean and slow drifts.
5. **Window:** multiply by a Hann window so the edges of the 10 s do not
   create false frequencies.
6. **Spectrum with a direct DFT**, only for 40–180 BPM in 1 BPM steps:
   `power(f) = (Σ x·cos 2πf·t)² + (Σ x·sin 2πf·t)²`. We need no FFT crate.
   **As built:** the 10 s are first resampled onto an even 20 Hz grid (200
   points, linear interpolation), which takes care of uneven frame times
   and dropped frames once. On the even grid, the angle turns by the same
   step from sample to sample, so a rotating pointer (one complex multiply
   per sample) replaces sin/cos: `libm` is called only twice per frequency.
   About 28k multiply-adds in total. `pump()` every 16 frequencies. Checked
   in Python on a synthetic signal (72, 55, 110, 160 BPM with drift, noise,
   jitter and a missing frame): found within 0.2 BPM.
7. **Heart rate:** the frequency with the most power. A parabola through the
   peak and its two neighbours gives a finer value. It is shown once the
   buffer holds 10 s.

Why a straight line and not a 1 s moving average (your question): you were
right to worry. Subtracting a 1 s moving average is itself a filter, and
its effect falls inside our band. It removes about 40% of the signal at
40 BPM and adds about 20% near 90 BPM, so it can move the peak. A straight
line over 10 s only removes changes slower than about 6 BPM, far below 40.

**Window length and resolution:** resolution = 1 / window length.

| `HR_WINDOW` | Resolution | Follows changes |
| --- | --- | --- |
| 5 s | 12 BPM | fastest, but too coarse |
| 8 s | 7.5 BPM | in between |
| **10 s (default)** | **6 BPM** | the slowest of the three, but the most stable |

The 1 BPM steps draw a smooth curve but add no information. The parabola
still places a clean peak better than the resolution. `HR_WINDOW` is one
constant, so trying 8 s is a one-line change. A median of the last few BPM
values makes the number steadier.

Needed FPS: at least 2 × 3 Hz = 6 FPS (Nyquist). 15+ FPS is better.

## 6. Camera change: lock auto exposure and white balance

This is the only change outside our folder.

**Why lock and not just switch off:** when the camera starts, we do not
know the light yet. So: auto adjustment stays on at the start, then, once
the face fills the view and the image has settled, we **freeze** it. The
sensor then keeps the exposure and colour gains it chose last.

**Registers** (page 0 of the GC0308, as Espressif's `esp32-camera` driver
uses them; **to check** with the datasheet and on the board):

- auto exposure (AEC) on/off: register `0xd2`, bit 7 (`0x80`). The start-up
  program writes `0x90`, so it is on.
- auto white balance (AWB) on/off: register `0x22`, bit 1 (`0x02`). The
  start-up program writes `0x57`, so it is on.

Clearing a bit freezes that part. Setting it again switches it back on.
`Registers::update_bits` already does this kind of write (used in
`gc0308.rs` for the orientation).

**The problem: who can talk to the sensor after start-up?** The sensor's
settings bus is the board's shared I2C bus. `Board::init` uses it on CPU0
once, then hands it to CPU1. From then on, only CPU1 tasks use it
(`src/board/i2c.rs`). The `Camera` handle on CPU0 has no bus.

**Decision: lock at any time, with a small task on CPU1** (chosen over
"lock once at start-up", which would lock to the light at power-on):

- `Camera::set_auto_adjust(false)` sends a request to a small new task on
  CPU1. The task writes the two bits. `true` switches auto adjustment on
  again.
- This is the same pattern as the backlight: a handle on CPU0, a task on
  CPU1, and a `Signal` between them (`src/capabilities/backlight/`).
- Our app locks 3 s after start-up. A tap on the screen unlocks, waits
  3 s for the image to settle, and locks again: tap once your face fills
  the view. (With face detection, the lock can later follow the face.)
- Files: `camera/mod.rs`, `camera/gc0308.rs`, a new `camera/runtime.rs`,
  and about 8 lines in `src/board/mod.rs` and `src/board/cpu1.rs` to
  create and start the task (approved). The task starts only when the
  board has a camera.
- The dashboard shows the state under the plot: "exposure auto, lock in
  Ns" or "exposure locked, tap=redo".

**Backward compatible:** the start-up program does not change, and auto
adjustment stays on. Only an app that calls the new function sees a
difference. The demo and every other app behave exactly as before.

Open detail: the start-up code talks to the sensor at 100 kHz, but the
CPU1 bus runs at 400 kHz. First try: 400 kHz. After each change, the task
reads both registers back and logs them
(`Camera auto adjust locked: AEC 0x.., AWB 0x..`). Expected: AEC `0x10`
and AWB `0x55` when locked, `0x90` and `0x57` when on (if nothing else
changed these registers since start-up). If the log shows an I2C error
instead, the task has to switch the bus to 100 kHz for these writes.

## 7. Where the per-pixel work happens

The display asks our `ScanlineSource` for one row at a time (`fill_row`),
while its DMA sends the previous rows. In that callback, for each camera
row we:

- copy the middle 160 pixels into the panel row (like `CenteredCrop`),
- add the green values of the ROI pixels in this row to a sum (for now all
  160 pixels).

With face detection (steps 5–7), also:

- draw the face box: overwrite a few pixels of the row with a colour,
- write every 2nd pixel of every 2nd row into a small grey image (80x120)
  for the detector.

Everything happens in the one pass that is needed anyway to send the image.
`while_transferring` calls `frame.pump()`, like the demo.

## 8. Files

```text
src/bin/00_heart_rate_face_cv/
├── PLAN.md          this file
├── CLAUDE.md        points Claude at this plan
├── main.rs          Board::init, the loop, FPS, drop counter, camera lock (tap), timing
├── view.rs          left half: the ScanlineSource (crop, green sum; later box, grey image)
│                    and green_mean() for frames that are not shown
├── dashboard.rs     right half: own PSRAM image, FPS, signal plot, status, HR, spectrum
├── signal.rs        20 s ring buffer; on the last 10 s: resample, line fit, window, DFT, peak
├── face/
│   ├── mod.rs       a common `Detector` interface: grey image → Option<box>
│   ├── skin.rs      option A
│   ├── lbp.rs       option B: integral image + cascade evaluation
│   └── cascade.rs   option B: GENERATED table (do not edit by hand)
└── tools/
    └── lbp_to_rust.py   OpenCV XML → cascade.rs
```

Cargo ignores the `.md` and `.py` files. `main.rs` makes this folder one
binary, like `src/bin/demo/`.

Outside the folder (the exception, section 6):

```text
src/capabilities/camera/
├── mod.rs        + the public set_auto_adjust function (and the request channel)
├── gc0308.rs     + the two register bits and a function that writes them
└── runtime.rs    NEW: the CPU1 task
src/board/mod.rs, src/board/cpu1.rs   ~8 lines to create and start the task
```

## 9. Steps

Each step ends with something visible on the board.
Build: `cargo dist --bin 00_heart_rate_face_cv`.

0. ✅ **Skeleton.** `main.rs` from `template.rs`, drawing one colour. Check
   that the binary name works (constraint 6). Done: builds, runs, clippy
   passes.
1. ✅ **Camera on the left + FPS.** `view.rs` with only the crop. `Canvas` of
   160x240 for the right half, showing only the FPS. Measure the FPS and
   watch the log for dropped-frame warnings. Done, see "Measured" below.
2. ✅ **Green signal of the whole view.** The green mean of all 160x240
   pixels per frame, the ring buffer, and the plot on the top right. Every
   frame is a sample (20 per second). Only every `RENDER_EVERY`th frame
   (3) is shown; the others are only read for green. The dashboard is
   redrawn at 2 Hz, with its own PSRAM image. A timing panel in the bottom
   half shows where each second goes. Done, 20 FPS without drops, see
   "Measured".
3. ✅ **Lock exposure and white balance** (section 6), moved **before** the
   heart rate. The step 2 photo shows jumps of ~20 green levels, and the
   pulse is well under 1 level: without the lock, the spectrum would show
   the jumps, not the pulse. Done: the writes work at 400 kHz, the log
   shows `AEC 0x10, AWB 0x55` when locked and `0x90, 0x57` when on. The
   readjustments seen while locked came with "on" log lines: accidental
   taps while holding the board. Not yet confirmed with the datasheet
   that these two bits stop **all** automatic brightness changes (the AWB
   register has 4 more "auto" bits). Idea for later: a long press instead
   of a tap.
4. ✅ **Heart rate.** Resampling to 20 Hz, line fit, Hann window, DFT on
   the last 10 s, spectrum plot, BPM number, in the bottom half (it
   replaces the timing panel). A footer line shows the DFT time and the
   dropped frames since start-up. Done: works "pretty decent" when the
   face fills the view and the head stays still. Movement or leaving the
   frame spoils it for the next ~10 s (the window length).
Also done alongside: learning notes in `human-learning/` (02–06, the
concepts of this project) and a slide deck to present it (5–10 minutes).

5. **Face detection A (skin).** A box follows the face, smoothed over time.
   The ROI comes from the box. Brings back the box outlines and the ROI
   code of the first step 2 version.
6. **Face detection B (LBP).** Converter script + `lbp.rs`. Measure the time
   per detection and call `pump()` inside. Compare with A. Keep the better
   one, or use B with A as a fallback.
7. *(Optional)* **Tiny CNN (D)**, only if B is not good enough.

## Measured

**Step 1: 20 FPS.** What was running:

- CPU0 (our app): each frame, the middle 160x240 of the camera to the left
  half (~15 ms on the SPI bus, the camera is pumped meanwhile). Once per
  second, the FPS text on the right half (`Canvas::show`, only a few changed
  pixels). The loop never sleeps.
- CPU1 (library, always on, even when the app does not use them): ESP-NOW
  radio, audio, IMU at 100 Hz, touch, light and proximity sensor.
- Memory: 457 KiB of PSRAM used before CPU1 starts (mostly the 3 camera
  frame buffers of 150 KiB each), plus 150 KiB for the dashboard canvas.

What it means:

- 20 FPS = 50 ms per frame, but the display needs only ~15 ms of it. So the
  limit is most likely the sensor's own frame rate, not our code. The rest
  of each 50 ms is spent waiting in `Frame::finish` for the next frame.
  That waiting time is the budget for face detection later.
- 20 FPS is well above the 6 FPS that 180 BPM needs. 10 s of data are
  about 200 samples.
- One "Camera frame dropped: the DMA ring overflowed" warning at start-up.
  The camera logs only the 1st and then every 32nd dropped frame. So no
  further warning means fewer than 32 more drops, not zero. The FPS value
  is the better check: dropped frames lower it.

**Step 2, first version: 12.5 FPS, and several "frame dropped" warnings.**
That version had a fixed face box and drew the dashboard with `Canvas`,
4 times per second. Cause: `Canvas::show` sends the changed plot without
pumping the camera. The ring overflows, and the camera needs about 2 frame
periods to start again. 4 redraws × ~2 lost frames ≈ 8 frames per second,
and 20 − 8 = 12. Fix: the dashboard draws into its own image in PSRAM and
sends it with `render_from`, whose `while_transferring` pumps the camera.
It also pumps between drawing steps.

**Step 2, whole view + own dashboard image: 8–9 FPS, ~1 dropped frame per
shown frame.** The on-screen timing (ms per second) showed: `begin` 582
(restarting the capture after drops), `render` 194 (≈ 24 ms per frame,
but the SPI needs only ~15 ms), `finish` 124, `dashboard` 148. Cause: the
display calls `while_transferring` only while the CPU **waits** for the
previous batch. With the green sum (and the dashboard's pixel
conversion), filling a batch took longer than sending it. So there was no
wait, no pump, and the ring overflowed during every render. Change:
`pump()` at the start of every `fill_row` as well. This was right, but it
was **not enough**: still 9 FPS, still mostly `begin`.

Lesson for later steps: never count on `while_transferring` alone. Any
code that works on rows must pump by itself.

The photo also showed the signal jumping between ~14 and ~36 (green
levels, 0–63) within 20 s: movement and auto exposure. This is why the
camera lock moved before the heart rate (step 3).

**Step 2, final: 20 FPS, no drops.** Changes: show only every 3rd frame
(`RENDER_EVERY = 3`, the others are only read for green), dashboard at
2 Hz instead of 4, and the dashboard image cleared in slices of 20 rows
with a pump after each. Timing (ms per second):

| Part | `RENDER_EVERY = 3` | `RENDER_EVERY = 1` | Per call |
| --- | --- | --- | --- |
| `begin` | 0 | 0 | – |
| `render` | 188 | 561 | ~28 ms per shown frame |
| `green` | 217 | 0 | ~16 ms per frame read for green only |
| `finish` | 527 | 371 | waiting for the sensor |
| `dashboard` | 115 | 114 | ~57 ms per redraw |
| drops (all three) | 0 | 0 | |

What it means:

- Showing every frame also works (`RENDER_EVERY = 1`: 20 FPS, no drops).
  So rendering was never the cause. What changed in the dashboard was the
  cause: most likely the one-piece clear of its 77 KB image. At ~5 MB/s
  (constraint 7), that is ~15 ms without a pump, 3 times what the camera
  ring holds. (Not proven on its own: 2 Hz instead of 4 changed at the
  same time.)
- The sensor is the limit again: more than half of each second is spent
  waiting in `finish`.
- Budget for later work (face detection): ~26 ms per frame with
  `RENDER_EVERY = 3` (`finish` / 20 frames), ~18 ms with 1. We keep 3.

## 10. Risks and open questions

- ~~Dropped frames during `Canvas::show`~~: it happened (12 FPS). Fixed in
  step 2 with our own image and `render_from` (see "Measured").
- ~~Camera FPS unknown~~: measured 20 FPS in step 1 (see "Measured").
- **The register bits for AE/AWB** come from another driver, not from the
  datasheet. Step 3 checks them on the board. Even when locked, light that
  flickers (some LED lamps) or changes still disturbs the signal.
- **10 s reacts with a delay:** a change in heart rate shows fully only
  after about 10 s. Use 8 s if that feels too slow (`HR_WINDOW_S` in
  `signal.rs`).
- **Accidental taps** unlock the camera while the board is held (step 3).
- **The signal is tiny:** the pulse changes the colour by far less than 1%.
  Expect a noisy result. Movement and talking ruin it.
- **Licence:** the OpenCV cascade files come with OpenCV's BSD-style
  licence. Keep the licence note in the generated `cascade.rs`.
