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

        bounded snapshots / signals / channels cross between cores
```

`resources.rs` is the bootstrap ownership map. Raw peripherals are moved once to
the core/service that owns them. CPU0 owns the LCD. CPU1 owns timing-sensitive
acquisition and radio services.

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
- `models` does not know screen geometry, line counts, fonts, or renderer types.
- `touch` publishes physical touch coordinates; navigation hit-testing belongs to
  `ui::navigation`.
- Physical panel dimensions are board facts in `board`; presentation geometry is
  centralized in `ui::layout`.

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
3. Add drawing code in `ui/views.rs`, or a focused `ui/<view>.rs` module if the
   renderer becomes substantial.
4. Keep the LCD driver generic: submit a framebuffer region or use
   `Display::render_scanlines()` for a justified partial/high-rate path.
5. Validate hardware behavior plus `dalloc/dfree`, internal heap, and CPU0 stack
   diagnostics before merging.

Do not introduce a second retained UI runtime or hide dynamic allocation inside
view rendering. If future product requirements genuinely need a richer UI
framework, treat that as an explicit architecture decision rather than layering
it beside this renderer.
