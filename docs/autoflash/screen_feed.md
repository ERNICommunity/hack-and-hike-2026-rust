# Brief: live screen feed over USB, for autoflash

This document instructs a fresh Claude Code session. It contains every
decision that was made while designing the feature, so nothing has to be
re-derived. Read it fully before touching a file, then implement all of it.

## Goal

Presenters want to show the board's 320x240 screen on a laptop. Build a
cheap live feed of the panel over the USB cable that is already plugged in,
and show it in the autoflash browser page, in the upper-right corner of the
device tab. The feed is part of the logging system: it is always on, in
every application, with no application code involved.

## Environment and rules for this repository

- Dev container. `CARGO_TARGET_DIR=/tmp/cargo-target` is set. The repository
  root's `.cargo/config.toml` builds for `xtensa-esp32s3-none-elf` with
  `build-std`. Host-side tools (`tools/autoflash/`, `tools/facekit/`) are their own
  Cargo workspaces with their own `.cargo/config.toml`; always run Cargo from
  inside their folder.
- Host tests of the firmware logic: `./scripts/test.sh` (runs
  `crates/core` tests on the host).
- The user flashes the board manually. Never run espflash, never flash, never
  start the autoflash server unasked. At the end, hand over the command
  `cargo dist --bin demo` (run in the repository root; it writes
  `firmware.bin`, which autoflash picks up).
- `tools/autoflash/` may be read and edited for this task (an earlier rule said
  not to; the user lifted it for this feature).
- Documentation style of this repository: plain English, short sentences,
  every abbreviation expanded on first use, every item has a doc comment
  (`#![warn(missing_docs)]` and `clippy::missing_docs_in_private_items`
  are on). `unsafe_op_in_unsafe_fn` and `clippy::mem_forget` are denied,
  `clippy::large_stack_frames` is denied: keep large buffers out of task
  stacks (CPU1 tasks share one 16 KiB stack).
- The esp-hal sources for the pinned git revision are checked out at
  `/usr/local/cargo/git/checkouts/esp-hal-*/dbd951f/` (esp-hal, esp-println,
  esp-backtrace, esp-radio). Read them for driver details.
- Commit messages end with `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>`.
  Commit only when the user asks.

## Facts established about the code

- Every pixel that reaches the panel passes through `Transport::render` in
  `src/capabilities/display/transport.rs`. It sets a window for a
  `Rectangle` in panel coordinates and sends rows in batches of
  `BATCH_LINES = 7` rows. For each batch it fills the free DMA buffer with
  big-endian RGB565 bytes (`source.fill_row`) and then calls `send`. The
  filled bytes are readable right before `send`. The canvas, the demo's
  navigation rail (`render_scanlines`) and the camera (`render_from`) all
  go through this function.
- `src/ui/canvas.rs` already sends only changed rows, grouped into windows.
  So for UI screens the mirrored traffic is small. The camera screen sends
  full 276x240 frames at panel rate.
- Logging today: `src/logging.rs`. The `log::Log` impl formats each record
  into a 512-byte `ArrayString` and calls `esp_println::println!` (the only
  `println` use in `src/`). esp-println is configured with the
  `jtag-serial` feature: the board's USB-C is wired to the ESP32-S3's native
  USB Serial/JTAG peripheral. The log history in PSRAM (`LogHistory`,
  `hack_and_hike_core::lines`) must stay.
- Boot order in `Board::init` (`src/board/mod.rs`): heaps, `logging::init`,
  `esp_hal::init`, `psram::enable`, `logging::enable_history`, RTOS timer,
  I2C recovery, power and resets, camera bring-up, `display::init`, codecs,
  light sensor, then `cpu1::start(...)` with a `Cpu1` struct of resources.
- `src/board/cpu1.rs`: CPU1 runs one embassy executor. Peripherals that
  need an interrupt on CPU1 are converted to async there (see
  `i2c::into_async`). Capability runtimes follow the pattern
  `pub(crate) fn spawn(spawner: &Spawner, ...)` with `#[embassy_executor::task]`.
- esp-hal's USB Serial/JTAG driver: `esp_hal::usb::usb_serial_jtag::UsbSerialJtag::new(peripherals.USB_DEVICE)`,
  `.into_async()` binds the interrupt on the current core, `.split()` gives
  `UsbSerialJtagRx` and `UsbSerialJtagTx`. The async `Tx` implements
  `embedded_io_async::Write`: it writes 64-byte chunks and awaits the
  "IN empty" interrupt after each chunk. When no host reads the port, the
  write waits forever (that is fine, see below). The async `Rx` implements
  `embedded_io_async::Read`.
- esp-backtrace 0.20 provides the panic handler and prints through
  esp-println. Its `custom-pre-backtrace` feature calls an
  `extern "Rust" fn custom_pre_backtrace()` that the firmware must define
  (`#[unsafe(no_mangle)]`), before it prints anything.
- embassy-sync 0.8 (already a dependency) has `pipe::Pipe`
  (`try_write`, `free_capacity`, async `read`) and
  `zerocopy_channel::Channel` (`new(&mut [T])`, `split()`, async `send()`
  returning `&mut T`, `send_done()`, async `receive()`, `receive_done()`).
- Autoflash (`tools/autoflash/`): a Vite + TypeScript page (`src/main.ts`, 1,800
  lines) served by a dependency-free Rust server (`server/`). The page uses
  Web Serial. `startMonitor(session)` opens the port and runs a read loop that
  decodes bytes with a streaming `TextDecoder` and calls
  `receiveSerialOutput(session, text)`, which feeds the backtrace collector
  and the log. `DeviceSession` is the per-device state. The terminal card is
  a grid of header, tab strip (`#terminal-tabs`) and `.terminal-body`
  (scrolling `pre#terminal-output`, `position: relative`). Constants live in
  `src/config.ts`. Tests are Vitest (`npm test`). The built page in `site/`
  is embedded into the server by `build.rs`; rebuild it with
  `npm ci && npm run build:selfhost` and commit `site/`. Node 20 and npm are
  installed in the container.

## Design decisions (final)

### Wire format

One USB Serial/JTAG port carries both the log and the screen feed.

- Log lines stay plain UTF-8 text with `\n`, exactly as today, so any
  terminal still shows them.
- A screen packet is: one `0x00` byte, the COBS-encoded body, one `0x00`
  byte. COBS (Consistent Overhead Byte Stuffing) removes every zero byte
  from the body, so `0x00` is an unambiguous delimiter. UTF-8 text never
  contains `0x00`; the logger replaces a `0x00` in a record with a space.
- Body layout (all integers little-endian):
  - byte 0: magic `0xFE` (never valid UTF-8, so text can never be mistaken
    for a packet),
  - byte 1: kind,
  - kind-specific fields,
  - last two bytes: CRC-16/CCITT-FALSE (polynomial 0x1021, init 0xFFFF, no
    reflection, no final XOR; test vector: `"123456789"` gives `0x29B1`)
    over every body byte before the CRC.
- Kind `0x01` Hello: `version u8 = 1`, `width u16`, `height u16`. Sent when
  the encoder task starts (boot) and after every refresh request. The host
  resets its image to black on Hello.
- Kind `0x02` Rect: `x u16`, `y u16`, `w u16`, `h u16`, then the pixel
  runs for `w * h` pixels in row-major order:
  - control byte `c < 128`: a literal run of `c + 1` pixels follows, each
    two bytes big-endian RGB565;
  - control byte `c >= 128`: a repeat run of `c - 127` copies (1 to 128) of
    the one pixel that follows (two bytes big-endian RGB565).
  Worst case expansion is under 1 %, so camera frames stay affordable.
- One Rect carries at most 768 raw pixels (1,536 bytes). Packet slot size
  on the device: 1,664 bytes (header, runs, CRC, COBS overhead of one byte
  per 254 plus one, and the two delimiters fit).
- Host to device: any received byte is a refresh request. Autoflash sends
  `R` (0x52). On a refresh the firmware sends Hello and marks the whole
  panel dirty.

### Firmware

`src/logging.rs` becomes the module folder `src/logging/`:

- `mod.rs`: what is there today (`LogHistory`, the `log::Log` impl,
  `init`, `enable_history`, `report_memory`) with one change: instead of
  `esp_println::println!`, the logger appends `\n` and calls
  `serial::push_text`. Also: `pub(crate) fn spawn(spawner: &Spawner, usb: USB_DEVICE<'static>)`
  that creates the async USB driver on CPU1 and spawns the three tasks
  below; and the panic hook
  `#[unsafe(no_mangle)] fn custom_pre_backtrace()` that sets the stop flag
  and busy-waits a few milliseconds (use `esp_hal::delay::Delay`), so the
  USB writer finishes its current 64-byte chunk and parks before
  esp-backtrace prints. Re-export what applications use today.
- `serial.rs` (crate-private): the USB side.
  - `TEXT: Pipe<CriticalSectionRawMutex, 8192>` static. `push_text(&str)`
    is all-or-nothing: if `free_capacity()` is smaller than the line, drop
    the line and count it. When a later push succeeds and the counter is
    non-zero, first push one line
    `[WARN] serial: N log lines dropped\n` and reset the counter. Both cores
    call `push_text`; the critical-section mutex makes that safe.
  - `PACKETS`: a `zerocopy_channel::Channel<CriticalSectionRawMutex, [u8; 1664], 2>`
    over storage in PSRAM (`psram::leaked_slice`) or a static, never on a
    stack. The encoder fills a slot in place; the writer sends it. Packets
    are never copied twice and never interleave with text.
  - `PANICKED: AtomicBool`. The writer checks it before every chunk and
    parks forever when set.
  - Writer task: loop over `select(TEXT.read(&mut buf64), PACKETS.receive())`.
    `select` polls the first future first, so text has priority. Text is
    written in pieces of at most 64 bytes. A packet is written whole, in
    64-byte chunks, then `receive_done()`. Use the driver's
    `embedded_io_async::Write`. When no host reads, the write blocks; text
    then drops with a count, and the encoder blocks on `send()`. Both
    resume when a host reads. The first bytes a late host receives are the
    buffered boot log; that is intended.
  - Reader task: `rx.read(&mut [u8; 8])`; every completed read signals
    `REFRESH` (an embassy `Signal<CriticalSectionRawMutex, ()>`).
- `mirror.rs` (crate-private): the screen side.
  - Shadow framebuffer: `&'static [AtomicU16]` of `WIDTH * HEIGHT` pixels
    (native u16, converted from the big-endian bytes), allocated in PSRAM by
    `pub(crate) fn enable()`, which `Board::init` calls right after
    `psram::enable` and before `display::init`. Store the reference in a
    `critical_section::Mutex<Cell<Option<&'static [AtomicU16]>>>`; before
    `enable`, `record` does nothing. Check that `core::sync::atomic::AtomicU16`
    exists for the Xtensa target; if not, use `portable_atomic::AtomicU16`
    (add the crate; it is already in the dependency tree via esp-hal).
    Relaxed loads and stores are enough; a torn rectangle is re-sent on the
    next tick because CPU0 marks the rows dirty again.
  - Dirty spans: `[Span; HEIGHT]` with `min`/`max` column (`u16`, empty when
    `min > max`), in a `critical_section::Mutex<RefCell<...>>`.
  - `pub(crate) fn record(x: usize, y: usize, width: usize, rows: usize, bytes: &[u8])`
    is called by `Transport::render` for every batch with the filled bytes
    (`rows * width * 2` bytes, big-endian). It stores the pixels into the
    shadow and widens the spans of rows `y..y+rows` to include
    `x..x+width`. Cost: a few microseconds per batch, overlapped with the
    DMA transfer of the previous batch.
  - Encoder task on CPU1: send Hello once at start. Then loop: wait for a
    40 ms `Ticker` tick or the `REFRESH` signal (on refresh: send Hello and
    mark all rows full-width dirty). Take a snapshot of the spans (copy into
    a static or PSRAM buffer, reset the live spans) under the critical
    section. Walk rows top to bottom; group consecutive dirty rows whose
    spans are exactly equal; split a group into Rect packets of at most
    `max(1, 768 / w)` rows. For each packet: `let slot = sender.send().await`,
    encode the rows from the shadow into the slot with the `crates/core`
    encoder, `send_done()`. The packet contents are read from the shadow at
    encode time, so they are current.
- `src/capabilities/display/transport.rs`: in `render`, after the
  `fill_row` loop and before `self.send(...)`, call
  `crate::logging::mirror::record(area.top_left.x as usize, area.top_left.y as usize + first_row, width, rows, &buffer[..rows * row_bytes])`.
  Add a sentence to the module docs: the transport reports every batch to
  the screen mirror in `logging`.
- `src/board/mod.rs`: call `logging::mirror::enable()` after
  `psram::enable`; add `usb: peripherals.USB_DEVICE` to the `Cpu1` struct.
- `src/board/cpu1.rs`: field `pub(super) usb: USB_DEVICE<'static>`; in
  `run`, `logging::spawn(&spawner, cpu1.usb)` (create the driver there, on
  CPU1, like the async I2C bus). Update the module docs and the CPU1 task
  list in `docs/architecture.md`.
- `Cargo.toml`: add `"custom-pre-backtrace"` to the esp-backtrace features.
  Keep esp-println (esp-backtrace prints through it); its `log-04` feature
  can be dropped because the firmware has its own logger.
- `crates/core/src/screen.rs` (new, `no_std`, no dependencies): COBS encode
  and decode, CRC-16/CCITT-FALSE, the pixel run encoder and a decoder (the
  decoder is for tests and mirrors the TypeScript one), `Hello` and `Rect`
  packet writers into a caller-provided buffer, the constants (magic, kinds,
  version, max pixels, slot size). Tests: CRC vector, COBS round trips
  including zeros at the start, end and 254-byte runs, pixel round trips
  (all equal, all different, runs longer than 128, a row split), a packet
  round trip, and one golden packet printed as hex so the TypeScript test
  can use the same bytes. Add the module to the table in
  `crates/core/src/lib.rs`.

### Autoflash

- `src/stream.ts` (new): `DeviceStreamParser` with `feed(bytes: Uint8Array)`
  and two callbacks, `onText(text: string)` and `onPacket(body: Uint8Array)`.
  Rules: split at `0x00`. A segment is undecided until it has two bytes;
  hold at most one byte. If byte 1 is `0xFE`, buffer the segment as a
  packet candidate, capped at 4,096 bytes (beyond that, flush as text).
  Otherwise stream the bytes to `onText` as they arrive, through one
  streaming `TextDecoder`. At the delimiter, COBS-decode a candidate,
  check the CRC and the magic, and call `onPacket` with the body without
  the CRC; on failure drop it silently and count it. `reset()` for a new
  monitor session. Empty segments (two delimiters in a row) are ignored.
- `src/screen.ts` (new): `ScreenMirror` per device: an `ImageData` and an
  offscreen `<canvas>`; `apply(body)` handles Hello (resize, clear to
  black, set `version`, `width`, `height`) and Rect (decode runs into the
  ImageData with bounds checks; ignore a rect that does not fit); one
  `putImageData` per animation frame, not per packet; counters for
  updates per second and bytes per second over the last second.
- `src/main.ts`: `DeviceSession` gets `stream: DeviceStreamParser` and
  `screen: ScreenMirror`. In `startMonitor`, reset the parser and, after the
  port is open and not flashing, write the refresh byte `R` through a
  short-lived writer on `port.writable` (guarded with try/catch; never while
  esptool-js owns the port). The read loop feeds raw bytes to the parser;
  `onText` calls the existing `receiveSerialOutput`, `onPacket` calls
  `session.screen.apply`. Keep the backtrace handling unchanged.
- The screen dock: a card anchored to the upper-right corner of the log
  area, below the tab strip, outside the scrolling element so it stays put
  while the log scrolls; wrap `.terminal-body` and the dock in a
  `position: relative` container. Give `pre#terminal-output` right padding
  while the dock is visible. Contents: the active device's canvas at 1x
  (320x240) by default, a 1x/2x toggle, a fullscreen button
  (`requestFullscreen` on the dock, scale to fit with
  `image-rendering: pixelated`), a refresh button (sends `R`), and one line
  with updates/s and KB/s. The dock is hidden until the active device has
  received its first screen packet, and hidden on the Autoflash tab. Each
  device keeps its own image; switching tabs swaps the canvas element.
- `src/config.ts`: `SCREEN_REFRESH_REQUEST = "R"`, `SCREEN_PACKET_MAGIC = 0xfe`,
  `SCREEN_MAX_PACKET_BYTES = 4096`, `SCREEN_DEFAULT_SCALE = 1`.
- Tests: `src/stream.test.ts` (text streams without waiting for a
  delimiter, text and packets mixed across chunk boundaries, a bad CRC is
  dropped, the golden packet from the Rust test decodes) and
  `src/screen.test.ts` (run decoding, Hello resize, out-of-bounds rect
  ignored). Node has no `ImageData`; keep the pixel decoding in a pure
  function over a `Uint8ClampedArray` so it is testable.
- Rebuild `site/` (`npm ci && npm run build:selfhost`) and run
  `cargo test` in `tools/autoflash/`. Commit the rebuilt `site/` together with the
  sources.
- `tools/autoflash/README.md`: a "Screen feed" section (what the dock shows, the
  refresh button, fullscreen, that it needs firmware with the feed, and the
  wire format in a few lines with a pointer to `crates/core/src/screen.rs`).

### Documentation in the repository root

- `README.md`, the "Log" paragraph in the capabilities section: mention that
  the same USB port also carries the live screen feed that autoflash shows,
  and that log lines stay readable in any serial terminal.
- `docs/architecture.md`: the logging row in the layers table, the CPU1
  task list (add USB writer, USB reader and screen encoder), a short
  "The screen feed" section (shadow buffer in PSRAM, dirty spans, the hook
  in the display transport, the priority of text over packets, the panic
  hook), and the memory section (150 KB shadow buffer in PSRAM; 8 KiB text
  pipe in internal RAM).

## Accepted trade-offs (do not re-litigate)

- Camera frames mirror at roughly 2 to 3 per second, limited by USB
  Serial/JTAG throughput (a few hundred KB/s). A moving image can show a
  tear line because the encoder samples the shadow while CPU0 writes the
  next frame. Acceptable for presentations.
- The panel copy costs CPU0 about 3 to 4 ms per full camera frame. UI
  screens see no measurable change.
- A late-connecting host first receives the buffered boot log and early
  packets, then the refresh brings the image up to date.
- After a panic, CPU1 keeps running but the USB writer stays parked, so log
  lines written after the panic are lost. Clean panic output matters more.

## Verification before handing over

1. `cargo build --release` in the repository root (all binaries, Xtensa).
2. `./scripts/test.sh` (core tests, including the new `screen` module).
3. `cd tools/autoflash && npm ci && npm test && npm run build:selfhost && cargo test`.
4. Report results faithfully. Then hand over `cargo dist --bin demo` and this
   hardware checklist for the user:
   - flash with autoflash; after the reset the device tab shows the dock
     with the demo's first screen;
   - the log lines still appear and are readable;
   - switch demo screens: the dock follows within a fraction of a second;
   - camera screen: image moves at a few frames per second, log still
     flows;
   - reload the autoflash page while the board runs: the image comes back
     after the refresh request;
   - `cargo dist --bin panic_backtrace`, tap to panic: the decoded backtrace
     still appears in the log with no garbage before it;
   - fullscreen button on a projector.
