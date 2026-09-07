# Architecture

Hack and Hike is structured around explicit ownership, bounded cross-core data,
and predictable presentation memory use on the ESP32-S3.

## Core ownership

```text
CPU0                                      CPU1
────────────────────────────────────      ────────────────────────────────
AppModel                                  Touch acquisition
Ui / presentation                         IMU acquisition + fusion
Display                                   Audio acquisition
  └─ SPI2 + DMA_CH1 + LCD                 ESP-NOW radio

             service-specific bounded contracts cross cores
```

`resources.rs` is the raw-hardware ownership map. Peripherals are moved exactly
once to the core/service that owns them. CPU0 owns the LCD. CPU1 owns the shared
runtime I2C bus, audio acquisition, and radio.

Board-level operations that span service boundaries are explicit bootstrap
steps. In particular, the AW9523 reset line policy resets the LCD and touch
controller together, so `main` performs that shared reset before CPU1 starts;
`display::init` does not reach through I2C to reset a CPU1-owned device.

There is deliberately no generic cross-core event bus. Each producer exposes the
smallest contract appropriate to its data:

- `touch` uses a bounded ordered channel for press/release edges and a latest
  signal for movement;
- `imu` publishes a replace-latest snapshot;
- `audio` exposes its specialized latest audio block contract;
- `network` publishes a replace-latest peer/service snapshot.

This keeps ordering, backpressure, and overwrite semantics explicit at each
service boundary instead of hiding them behind one abstraction.

## Presentation layers

The presentation path has three explicit responsibilities:

```text
models.rs
  application state, bounded snapshots, refresh cadence
      │
      ▼
ui.rs + ui/*
  navigation, layout, view composition, dirty rendering policy
      │ RGB565 regions / scanline closures
      ▼
display.rs + display/transport.rs
  LCD initialization, reusable scanline scratch, SPI-DMA transport
```

Dependency rules:

- `display` does not know `ViewId`, navigation icons, text, IMU, network, or log
  semantics.
- `ui` does not own SPI/DMA peripherals and cannot issue LCD controller commands.
- `models` does not know absolute screen placement, fonts, or LCD transport.
- `touch` publishes physical touch coordinates; navigation hit-testing belongs to
  `ui::navigation`.
- Physical panel dimensions are board facts in `board`; presentation geometry is
  centralized in `ui::layout`.

The presentation modules are intentionally small and role-specific:

```text
ui.rs                  coordinator / dirty dispatch
ui/layout.rs           authoritative UI geometry
ui/framebuffer.rs      fixed PSRAM RGB565 drawing surface
ui/navigation.rs       touch navigation + navigation rail pixels
ui/waveform.rs         partial high-rate microphone renderer
ui/views/mod.rs        view shell dispatch
ui/views/text.rs       Network + Log text views
ui/views/imu.rs        IMU attitude + compass view
```

## Display memory and rendering

There are two rendering strategies, both allocation-controlled:

1. **Buffered content views** — Network, IMU, Log, Sound, and static Microphone
   chrome render into one fixed 276×240 RGB565 framebuffer in PSRAM. The buffer
   is allocated once when `Ui` is constructed and never resized.
2. **Partial microphone updates** — the two 256×90 waveform canvases render
   directly through `Display::render_scanlines()`. This avoids sending the full
   content framebuffer at the ~32 ms waveform presentation cadence.

`Display` owns one 320-pixel internal-RAM scanline scratch buffer and two static
DMA TX line buffers. The transport overlaps preparation of the next line with
the previous SPI transfer.

The global allocator remains internal-RAM only. Large plain-data buffers and the
content framebuffer use the explicit PSRAM allocator. `memory.rs` continuously
tracks internal heap and stack headroom; navigation instrumentation verifies that
view changes remain allocation-flat.

## Model refresh policy

Only the active view refreshes presentation-sized state:

- Microphone: 32 ms
- IMU: 40 ms
- Network: 200 ms
- Log: 100 ms

CPU1 producers publish replace-latest snapshots where appropriate. CPU0 consumes
bounded values and never blocks a producer while rendering.

## Adding a view

A new view should follow this sequence:

1. Add the semantic `ViewId` and any bounded model state in `models.rs`.
2. Add fixed geometry only when needed in `ui/layout.rs`.
3. Add drawing code in the appropriate `ui/views/*` module, or create a focused
   module when the renderer becomes substantial.
4. Keep the LCD driver generic: submit a framebuffer region or use
   `Display::render_scanlines()` for a justified partial/high-rate path.
5. Validate hardware behavior plus `dalloc/dfree`, internal heap, and CPU0 stack
   diagnostics before merging.

Do not introduce a second retained UI runtime or hide dynamic allocation inside
view rendering. If future product requirements genuinely need a richer UI
framework, treat that as an explicit architecture decision rather than layering
it beside this renderer.
