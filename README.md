# ![Rust at ERNI Consulting header](assets/header.png)

## Hack and Hike 2026

Firmware for the **M5Stack CoreS3 Lite**, written in Rust for the ESP32-S3.

This repository is the starting point for a weekend of hacking. You are an
experienced programmer but new to Rust: everything you need is here, and you do
not have to understand the hardware drivers to build something that works.

The idea is simple:

> **Your application is one file. It takes the hardware it needs and runs a loop.**

The hardware comes as **capabilities**: small Rust APIs, one per function of
the device.

| Capability | Handle | What you get |
| --- | --- | --- |
| Display | `Display` | Draw pixels inside a rectangle |
| Backlight | `Backlight` | Set the screen brightness |
| Touch | `Touch` | Press and release events with a position |
| IMU | `Imu` | Orientation, acceleration, rotation, magnetic heading |
| Microphone | `Microphone` | 16 kHz stereo PCM blocks |
| Speaker | `Speaker` | Play 16 kHz stereo PCM |
| Network | `Network` | Send your own message types to nearby devices (ESP-NOW) |
| Camera | `Camera` | RGB565 frames |
| Log | `LogHistory` | Everything your code logged, for showing on screen |

## Contents

- [Build](#build)
- [Run the tests](#run-the-tests)
- [Your first application](#your-first-application)
- [Create your own application](#create-your-own-application)
- [The capabilities](#the-capabilities)
- [The three built-in applications](#the-three-built-in-applications)
- [Project folders](#project-folders)
- [Where does my code go?](#where-does-my-code-go)
- [Reading order](#reading-order)
- [Going deeper](#going-deeper)

## Build

Every application is a binary in `src/bin/`. Build one by name:

```bash
cargo build --release --bin imu_color
```

```bash
cargo build --release --bin color_ping
```

```bash
cargo build --release --bin demo
```

Without `--bin`, `cargo build --release` builds all of them. The first build
takes a few minutes; later builds take seconds.

## Run the tests

The hardware-independent logic (IMU math, the network protocol, the peer table)
lives in the crate `crates/core` and has ordinary Rust tests that run on your
computer:

```bash
./scripts/test.sh
```

The script exists because the repository's Cargo configuration targets the
ESP32-S3; it runs `cargo test` for that crate with your computer's target
instead. Put unit tests next to the code and scenario tests in
`crates/core/tests/`.

## Your first application

`src/bin/imu_color.rs` makes the whole screen a compass-calibration gauge: red
at 0 %, orange at 50 %, yellow at 75 % and green at 100 %, with a panel showing
the sensor's own numbers. This is its `main`:

```rust
#[esp_rtos::main]
async fn main(_spawner: Spawner) -> ! {
    let Board {
        mut display,
        mut imu,
        ..
    } = Board::init();

    let full_screen = Region::new(0, 0, WIDTH, HEIGHT);
    let mut screen = GuiSurface::new(WIDTH, HEIGHT);
    let mut shown = None;
    let mut last_redraw = Instant::now();

    screen.present_custom(&mut display.surface(full_screen), |frame| draw(frame, None));

    loop {
        if let Some(sample) = imu.latest() {
            let reading = Reading::from_sample(&sample);
            if shown != Some(reading) && Instant::now() - last_redraw >= REDRAW_PERIOD {
                shown = Some(reading);
                last_redraw = Instant::now();
                screen.present_custom(&mut display.surface(full_screen), |frame| {
                    draw(frame, Some(reading));
                });
            }
        }

        Timer::after(Duration::from_millis(10)).await;
    }
}
```

Line by line:

- The file starts with `#![no_std]` and `#![no_main]`: this is firmware. There
  is no operating system and no C-style `main`. You still have structs, enums,
  `Option`, iterators, closures and modules. The `esp_app_desc!()` line writes
  a small descriptor the bootloader expects; every application has it.
- `#[esp_rtos::main] async fn main(...) -> !` is an async entry point that
  never returns (`!`). The board runs your loop forever.
- `Board::init()` powers up the whole board and returns one handle per
  capability. The pattern `let Board { mut display, mut imu, .. } = ...` keeps
  the two handles this application needs and drops the rest. Dropping a handle
  is fine: the sensors keep running on the second CPU core.
- `imu.latest()` returns `Some(sample)` when a new sample arrived since the
  last call and `None` otherwise. Nothing blocks.
- `display.surface(region)` borrows the display for one rectangle; nothing can
  draw outside it. `GuiSurface` is a framebuffer the size of that rectangle:
  `present_custom` hands your closure a `frame` to draw on and then copies it
  to the panel in one go.
- The `draw` function below `main` fills the background and writes the text,
  using the helpers in `hack_and_hike::ui`. It is a plain function, so nothing
  about it is specific to this application.
- The screen is only redrawn when a value changed and at most four times a
  second, because the sensor publishes a hundred samples per second.
- `Timer::after(...).await` pauses this loop and lets other work on this core
  run. Every loop needs an `.await` somewhere.

The simplest way to draw is without a framebuffer at all:

```rust
let mut surface = display.surface(full_screen);
surface.render_scanlines(|_y, pixels| {
    pixels.fill(0x07E0); // green, as an RGB565 value
});
```

## Create your own application

Copy `src/bin/imu_color.rs` or `src/bin/color_ping.rs` to a new file, for
example `src/bin/my_hack.rs`, and edit the loop:

```rust
#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use hack_and_hike::Board;

esp_bootloader_esp_idf::esp_app_desc!();

#[esp_rtos::main]
async fn main(_spawner: Spawner) -> ! {
    let Board {
        mut display,
        mut touch,
        mut imu,
        ..
    } = Board::init();

    loop {
        // Read input.
        // Update your state.
        // Draw when something changed.

        Timer::after(Duration::from_millis(10)).await;
    }
}
```

Build it:

```bash
cargo build --release --bin my_hack
```

That is all. Cargo finds every file in `src/bin/` by itself, and CI builds
every binary. When the application grows, turn it into a folder,
`src/bin/my_hack/main.rs` plus sibling modules, like `src/bin/demo/`.

Keep the rules of your application (what a touch means, what a message means,
which colour is which) in your application. The capabilities stay generic.

## The capabilities

One snippet each. The handles come from `Board::init()`.

**Display.** Draw a rectangle row by row. `Pixel` is a 16-bit RGB565 value;
`hack_and_hike::ui::theme::pixel` has the project palette.

```rust
let mut surface = display.surface(Region::new(0, 0, WIDTH, HEIGHT));
surface.render_scanlines(|y, pixels| {
    pixels.fill(if y < HEIGHT / 2 { 0x001F } else { 0xFFFF });
});
```

**Touch.** Events queue up until you read them.

```rust
while let Some(edge) = touch.next_edge() {
    if let TouchEdge::Pressed(point) = edge {
        // point.x and point.y are display coordinates.
    }
}
```

**IMU.** The newest fused sample, about 100 times per second.

```rust
if let Some(sample) = imu.latest() {
    let roll = sample.orientation.roll_deg;
    let heading = sample.orientation.yaw_deg;
    let calibrated = sample.mag_status == MagStatus::Ready;
}
```

**Microphone.** 32 ms blocks of interleaved stereo samples (left, right, ...).

```rust
let mut block = [0i16; audio::SAMPLES_PER_BLOCK];
if let Some(info) = microphone.try_read(&mut block) {
    let loudest_left = info.peak_left;
}
```

**Speaker.** Queue a little audio on every loop iteration; the board plays
silence when the queue runs empty. See `TonePlayer` in `src/bin/color_ping.rs`.

```rust
let frames = speaker.available_frames().min(chunk.len() / audio::CHANNELS);
speaker.write(&chunk[..frames * audio::CHANNELS]);
```

**Network.** Define your own message type; the network only moves bytes.
Every board in the room shares one channel, so give the type a name that is
unique to your application: only messages with that name decode as `Hello`.

```rust
#[derive(Serialize, Deserialize)]
struct Hello {
    number: u32,
}

impl Message for Hello {
    const NAME: &'static str = "team-otters.hello";
}

if network.broadcast(&Hello { number: 42 }).is_err() {
    log::warn!("send queue is full, try again next loop");
}

while let Some(message) = network.next_message() {
    if let Ok(hello) = message.decode::<Hello>() {
        let reply = Hello { number: hello.number + 1 };
        if let Err(error) = network.send_to(message.sender, &reply) {
            log::warn!("reply not sent: {error}");
        }
    }
}
```

**Camera.** `camera` is an `Option`: `None` when no camera answered at boot.
A frame is a `ScanlineSource`, so it can be drawn directly.

```rust
if let Some(camera) = camera.as_mut()
    && let Some(mut frame) = camera.begin_frame()
{
    display.surface(full_screen).render_from(&mut frame);
    frame.finish();
}
```

**Backlight.**

```rust
if let Some(dim) = Brightness::new(30) {
    backlight.set(dim);
}
```

**Log.** Use the `log` macros anywhere; the demo's Log screen shows the
history through `LogHistory`.

```rust
log::info!("button pressed at {}", point.x);
```

## The three built-in applications

| Binary | Uses | What it shows |
| --- | --- | --- |
| `imu_color` | display, IMU | The smallest possible application (above) |
| `color_ping` | display, touch, network, speaker | One loop that combines four capabilities |
| `demo` | everything | A screen per capability with navigation |

**Color Ping** splits the screen into four colour bands. Tapping a band
broadcasts that colour; every other device that receives it plays a 300 ms
tone. A band lights up while you hold it, and on the receiving device while
its tone plays. Flash it to two devices and tap.

```mermaid
sequenceDiagram
    participant A as Device A
    participant Radio as ESP-NOW
    participant B as Device B

    A->>A: user taps Blue
    A->>Radio: ColorPing { Blue }
    Radio->>B: broadcast
    B->>B: decode ColorPing
    B->>B: play 1200 Hz for 300 ms
```

It is deliberately one loop: touch, network, audio and drawing each advance a
little on every iteration, and nothing blocks. That is the pattern to copy.

**Demo** is the full firmware: Network, IMU, Microphone, Speaker, Camera,
Settings and Log screens behind a navigation rail. Every screen implements the
same small `Screen` trait. `src/bin/demo/screens/settings/` is the one to copy
when you add a screen: a KDL layout file, a slider, and one capability handle.

## Project folders

```text
crates/core/        hardware-independent logic with tests (IMU math, network protocol)
src/
├── lib.rs          the library every application uses
├── bin/            the applications: demo/, imu_color.rs, color_ping.rs
├── board/          Board::init(): power-up order and the CPU1 runtimes
├── capabilities/   one module per capability: the APIs you call
├── platform/       facts about the PCB: pins, power rails, I2C bus
├── support/        logging with on-device history, PSRAM helpers
└── ui/             palette, drawing helpers, slider widget, embedded-gui glue
```

## Where does my code go?

| I want to... | Put it in... |
| --- | --- |
| Build a new device experience | `src/bin/my_app.rs` |
| Add a screen to the demo | `src/bin/demo/screens/` |
| Change the demo's navigation rail | `src/bin/demo/navigation.rs` |
| Add a reusable drawing helper or widget | `src/ui/` |
| Expose a new hardware operation | the matching `src/capabilities/...` module |
| Change how a sensor is configured | the matching capability |
| Change pins, power or reset wiring | `src/platform/` |
| Change the power-up order | `src/board/` |
| Add logic that should have tests | `crates/core/` |

A rule of thumb:

> If the code says **what the device should do**, it belongs in an application.
>
> If the code says **how a piece of hardware works**, it belongs in a capability.

## Reading order

1. `src/bin/imu_color.rs`
2. `src/bin/color_ping.rs`
3. `src/lib.rs` and `src/board/mod.rs`
4. `src/bin/demo/main.rs`, then `src/bin/demo/screens/settings/`
5. one capability API, for example `src/capabilities/imu/mod.rs`
6. `src/capabilities/display/mod.rs`
7. drivers and `crates/core` only when you need them

## Going deeper

[docs/architecture.md](docs/architecture.md) explains how the firmware is put
together: what happens in `Board::init()`, what runs on which CPU core, how
drawing and the demo's screens work, memory and PSRAM, the camera path, the
Rust ideas you will meet, and the design rules behind the layout.
