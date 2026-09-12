# ![Rust at ERNI Consulting header](assets/header.png)

## Hack and Hike 2026

ESP32-S3 firmware for the M5Stack CoreS3 Lite.

## Architecture

Hack & Hike firmware is built from **capabilities** and exactly one **application**.

A capability owns a piece of hardware/runtime implementation and exposes a semantic API. The selected application owns the capability handles enabled for its build and decides how to combine them. Applications may be graphical or headless; a headless application is simply an application that does not enable the `display` capability.

At runtime, CPU0 runs the selected application. CPU1 runs the timing-sensitive capability runtimes such as IMU acquisition, touch polling, audio DMA, networking, and display-brightness I/O. Cross-core queues/signals and HAL details remain private to the capability modules.

### Capabilities

Capabilities live in `src/capabilities/`:

- `display` — LCD ownership and bounded RGB565 `Surface` rendering
- `touch` — semantic touch points and press/release events
- `imu` — acceleration, angular velocity, magnetic field, and fused orientation
- `mic` — signed 16-bit stereo PCM capture blocks
- `speaker` — signed 16-bit stereo PCM output
- `network` — ESP-NOW discovery and typed postcard messaging
- `camera` — RGB565 camera frames and scanline access

Applications should use these semantic APIs rather than HAL peripherals, DMA channels, board registers, or chip-specific driver types.

### Applications

Applications live in `src/applications/`. A firmware build selects one application at compile time.

The default `app-stock` application lives in `src/applications/stock/`. It owns the standard navigation, all stock screens, and all capability handles required by those screens. The IMU, microphone, speaker, network, camera, settings, and log screens are internal components of that one application; they are not independently selected applications.

`src/ui/` contains only presentation primitives that another graphical application may choose to reuse. Navigation and screen composition are application policy.

The repository also contains `app-idle`, a minimal headless application used to exercise capability-only compositions. If no application feature is selected, firmware falls back to the same idle behavior so individual capabilities can still be compiled independently.

## Build the stock firmware

The default feature set selects the complete stock application:

```bash
cargo build --release
```

Equivalent explicit selection:

```bash
cargo build --release --no-default-features --features app-stock
```

## Headless applications

Headless applications need no special framework. They simply omit `display` from their feature dependencies.

For example, this builds the idle headless application with IMU and networking enabled:

```bash
cargo build --release --no-default-features --features app-idle,imu,network
```

The application owns the returned `Imu` and `Network` handles while the corresponding hardware runtimes continue on CPU1.

## Create an application

A new application should be a small, explicit composition rather than an implementation of a framework trait.

1. Create a module such as `src/applications/my_hack/mod.rs`.
2. Add an application feature to `Cargo.toml` listing only the capabilities it needs.
3. Add that application to the build-time selection in `src/applications/mod.rs` and make it mutually exclusive with the other application features.
4. In the application's `run` function, take ownership of `firmware::Bootstrap` and move the enabled capability handles into whatever state/tasks the application needs.

For a graphical IMU/network experiment, a feature could look like:

```toml
app-my-hack = ["display", "touch", "imu", "network"]
```

For the same experiment without a display:

```toml
app-my-hack = ["imu", "network"]
```

Then build only that application:

```bash
cargo build --release --no-default-features --features app-my-hack
```

An application owns each capability handle exactly once. If several screens or internal tasks need information derived from one capability, the application decides how to route or share that state internally. The capability does not impose application-level navigation, scheduling, mixing, or multicast policy.

Adding an application should normally not require changes under `src/capabilities/`. If an experiment repeatedly needs to modify capability implementation details, that is a sign that the semantic capability API may be missing a useful operation.

## Rendering

Graphical applications render pixels directly. The display capability owns the physical LCD transport; the application borrows a bounded `Surface` for a region and renders into it. A `Surface` cannot draw outside its assigned physical region.

Applications are free to define their own screen layout or navigation. The stock application's 44-pixel navigation rail is not a firmware-global requirement.

The stock camera screen preserves the direct QVGA RGB565 path: it crops camera scanlines directly into LCD DMA batches and pumps capture of the following frame while display DMA is in flight. No intermediate application framebuffer is required for that path.

## Scheduling

Application code runs on CPU0 and uses cooperative async scheduling. Long-running CPU work must yield/await regularly so other CPU0 tasks can run. Hardware acquisition and other timing-sensitive runtime work remains isolated behind capability APIs on CPU1.
