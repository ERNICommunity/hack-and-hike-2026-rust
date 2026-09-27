# Plan: sand and water in a box (IMU + particles + sound)

Goal: the screen is a box full of grains. Tilt or shake the board and the
grains slide, fall and pile up like sand (or flow like water). While grains
move, the speaker plays a soft "shhh" drag noise: louder when more grains
move, silent when everything is still.

Rules for this project:

- Every file lives in `src/bin/01_2d_particle_physics_sim/`. Nothing outside
  it changes: no `Cargo.toml`, no library code, no new crates.
- Reuse the library (`hack_and_hike::...`) and the crates the project
  already has: `embedded-graphics`, `libm`, `arrayvec`, `embassy-*`, `log`.
- Small steps. After each step the app builds, runs on the board and shows
  (or plays) something.
- Build commands (in the container): `cargo fmt`,
  `cargo clippy --release -- -D warnings`,
  `cargo dist --bin 01_2d_particle_physics_sim`, then flash `firmware.bin`.

## 1. What we reuse

| Need | Existing piece | Where |
| --- | --- | --- |
| Tilt and shake | `imu.latest()` → `acceleration_m_s2: Option<[f32; 3]>`, 100 Hz | `src/capabilities/imu/mod.rs` |
| Sound | `speaker.available_frames()`, `speaker.write(&[i16])`, 16 kHz stereo | `src/capabilities/audio/mod.rs` |
| Draw the box fast | `display.surface(rect).render_scanlines(\|y, row\| ..)` | `src/capabilities/display/` |
| Status line text | `ui::Canvas` + `ui::common::text`, `theme::*` | `src/ui/` |
| Buffers off the stack | `psram::leaked_slice` | `src/board/psram.rs` |
| sqrt etc. | `libm` | dependency |
| Program skeleton | `template.rs` | `src/bin/template.rs` |

The board has an accelerometer **and** a gyroscope (BMI270). We use only
the accelerometer: it already measures what the grains feel (gravity from
tilting plus the push from shaking). The gyroscope measures rotation speed,
which a box of sand does not care about.

## 2. The key idea: what the grains feel

`acceleration_m_s2` includes gravity and points **down** when the board is
still (`[0, 0, 9.8]` lying flat, screen up). Inside a moving box, a grain
feels exactly that vector: gravity minus the box's own acceleration. So we
use it directly as the grains' gravity, no extra maths.

IMU axes (the library's screen frame) → screen pixels:

```text
IMU x = out of the top edge   →  pixel y = −x   (pixels grow downwards)
IMU y = right across screen   →  pixel x = +y
IMU z = into the screen       →  ignored (lying flat: grains rest)
```

Step 0 checks these signs on the real board before anything else.

## 3. Screen layout (320x240)

```text
┌──────────────────────────────────────────┐ y=0
│ status: FPS · grains · motion · mode      │ 16 px, Canvas, 2 Hz
├──────────────────────────────────────────┤ y=16
│                                          │
│   the box: 80 x 56 cells of 4x4 px       │ 224 px, drawn row by row
│   each cell empty or one grain            │ from the grid
│                                          │
└──────────────────────────────────────────┘ y=240
```

Cell size is one constant (`CELL_PX`): 4 px gives chunky, visible grains
and a small grid (4,480 cells). Start with ~800 grains, tune later.

## 4. Simulation (the "pixel dust" method)

Each grain has a position and a velocity (floats, in cells). A grid says
which cell holds which grain, so collisions are one lookup, not a
comparison with every other grain. Fixed steps of 4 ms; every step:

1. `v += gravity · dt`, then a little drag (friction).
2. Speed limit: under 1 cell per step, so a grain never jumps over another.
3. Same cell or an empty cell: move.
4. Blocked: hop to the free neighbour (of 8) that is most **downhill**,
   then most along the motion. Sand only accepts cells within ~80° of
   gravity, so it keeps a slope; water accepts anything downhill.
5. Water only: if still blocked, look up to 8 cells sideways for a drop
   that is lower than here, and hop up to 3 cells towards it. This levels
   the surface.
6. Nowhere to go: bounce, `v = −v · restitution`.
7. Count grains that changed cell → **motion**, for the sound and status.

Every hop goes downhill, so the grains always come to rest (and the sound
stops). Sand vs water: drag, bounce, slide, "how downhill", flow on/off.

## 5. Sound: drag noise

Real sand sound is many tiny random clicks, which the ear hears as noise.

1. White noise from a tiny random number generator (xorshift, 3 lines).
2. A one-pole low-pass filter makes it a soft "shhh" instead of a hiss.
   Plus tiny random clicks (more when louder) for the grainy texture.
3. Volume = motion, smoothed (fast attack ~20 ms, slower release
   ~150 ms), so it never clicks. Below a small threshold: exact silence.
4. A maximum volume constant keeps it "not disturbing".

The speaker queue holds 1,024 frames = **64 ms**. We top it up every loop,
before and after drawing. A full-screen draw takes ~31 ms, so it fits, but
the loop must never stall longer than ~60 ms or the sound crackles.

## 6. Steps

| Step | Result on the board | Status |
| --- | --- | --- |
All steps were written at once (user's request) and checked in Python
first; none has run on the board yet.

| Step | Result on the board | Status |
| --- | --- | --- |
| 0 | Axes and signs: tilt right → grains go right; top edge up → grains go down. (No separate arrow screen; the grains show it.) | 🟡 written |
| 1 | Grains fall, bounce off walls. Status line: FPS, ms physics and draw per frame, moves/s. | 🟡 written |
| 2 | Grid collisions: piles and slopes (sand), level surface (water). | 🟡 written, Python-tested |
| 3 | Drag noise that follows motion, silent at rest. | 🟡 written |
| 4 | Long press (0.8 s) toggles sand/water; colours brighten with speed; 1-px gaps make sand grainy. | 🟡 written |

Files: `main.rs` (loop, IMU → gravity, touch, drawing, status), `sim.rs`
(grains and grid), `sound.rs` (drag noise).

## 7. Measured

**Python model of `sim.rs`** (host, 1,200 grains, 4 ms steps; moves/s =
grains changing cell per second; 0 = at rest = silent):

| Case | Sand | Water |
| --- | --- | --- |
| fall from the top, flat | settles, 0 | settles, 0 |
| tilt 30° / 45° | stays (holds its slope), 0 | surface turns level with gravity, 0 after ~3–6 s |
| tilt 60° / 90° | avalanches, 45° pile, 0 after < 3 s | flows, 0 after ~3 s |
| on its side, then flat again | 45° pile, 0 after < 3 s | still levelling after 6 s (~1,600 moves/s) |

Tuning history: v1 (only the cells next to the target) locked a packed
layer even at 60°; water with a "speed > 2.5" sideways rule never
levelled; putting hopped grains in the cell centre made water ~4× slower.

**On the board:**

| Version | FPS | Notes |
| --- | --- | --- |
| v1: grains + grid in PSRAM, 4 ms steps, colours looked up per pixel row | 7 | builds and runs; feels slow and laggy (the frame is so long that the 48 ms catch-up cap makes the simulation run slower than real time) |
| v2: grains + grid in internal RAM (heap `Vec`, ~28 KB), 6 ms steps, no sqrt for most grains, colours once per cell row | ? | to measure: FPS, phys ms, draw ms |

Why v2: PSRAM here runs over a 4-bit (quad) SPI bus. Every step reads all
1,200 grains and many grid cells, and every drawn row read the grid
again, so that data belongs in fast internal RAM. Big, rarely read buffers
(like the status `Canvas`) can stay in PSRAM.

## 8. Constraints

1. No big arrays on the stack: grains and grid live on the internal heap
   (`vec![..].leak()`, created once), because the physics reads them
   constantly.
2. No logging per frame; numbers go on the status line.
3. Every loop has an `.await` (a short `Timer::after`, no camera here).
4. Audio queue topped up at least every ~50 ms.
5. `cargo clippy --release -- -D warnings` passes.
