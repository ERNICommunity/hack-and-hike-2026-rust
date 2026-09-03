# Runtime ownership

The runtime follows one explicit ownership rule:

> **CPU0 owns application coordination, presentation, and display I/O. CPU1 owns non-display peripheral services and timing-sensitive acquisition/communication.**

```text
CPU0
├── application/model
├── Slint
├── LCD
├── SPI2 / display DMA
└── direct display overlays

CPU1
├── touch
├── audio
├── IMU
├── ESP-NOW
└── runtime system I²C
```

Hardware handles do not cross cores. Cross-core communication carries bounded,
typed data with semantics appropriate to each path.

## Cross-core data paths

The generic semantic lanes in `src/cross_core.rs` are intentionally small:

- CPU0 → CPU1: compact application commands.
- CPU1 → CPU0: compact semantic events.

High-rate or state-like data keeps specialized transport:

- touch press/release → bounded ordered channel;
- touch movement → latest value;
- audio waveform → specialized latest snapshot;
- future orientation/status → latest state rather than a sample stream.

Do not place raw audio blocks, network packets, IMU sample streams, hardware
handles, or other bulk payloads in the semantic command/event lanes.

## Board versus device ownership

`src/board/` owns PCB policy: power rails, reset lines, and enable signals.

Device configuration remains with the owning service:

- LCD controller configuration → `screen`;
- ES7210 configuration → `audio`;
- future BMI270 configuration → `imu`.

The generated theme remains sourced only from:

```text
theme.toml → build.rs → generated-theme.slint / theme_generated.rs
```
