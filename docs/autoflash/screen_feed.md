# The live screen feed: why it is built this way

The firmware sends a copy of the board's 320x240 screen over the USB
(Universal Serial Bus) cable that is already plugged in, mixed with the log
text. The autoflash page shows it beside the log. It runs in every
application, with no application code.

This page is the design record: the decisions behind the feature and the
costs that were accepted. It does not repeat how the feature works or how to
use it.

| What you want | Where |
| --- | --- |
| Use it: the dock, zoom, fullscreen, refresh | [`tools/autoflash/README.md`](../../tools/autoflash/README.md), "Screen feed" |
| How it works: the hook, the copy, the tasks | [`docs/architecture.md`](../architecture.md), "The screen feed" |
| The wire format, byte for byte | [`crates/core/src/screen.rs`](../../crates/core/src/screen.rs) |

## The goal

Show the board's screen on a laptop during a presentation, without extra
hardware and without a second cable. The board already has one USB-C socket,
wired to the ESP32-S3's USB Serial/JTAG peripheral, and the log already goes
through it.

## The decisions

**One port carries both the log and the screen.** A second channel would
need a second cable or a network. The port moves a few hundred KB per
second, which is enough for user interface screens and for a slow camera
image.

**Log lines stay plain UTF-8 text.** Any serial terminal still shows the
log, with or without autoflash. This was the constraint that shaped the
framing: the screen data has to be invisible to a plain terminal reader, not
the other way round.

**A packet is a `0x00` byte, the COBS-encoded body, and another `0x00`
byte.** COBS (Consistent Overhead Byte Stuffing) removes every zero byte
from the body, so `0x00` marks the packet edges without doubt. Text never
contains `0x00`; the logger replaces one with a space. The first body byte
is `0xFE`, which is never valid UTF-8, so a run of text can never be read as
a packet. A CRC (cyclic redundancy check, a checksum) ends the body, so a
packet damaged by a dropped byte is discarded rather than drawn.

**Pixels go out as runs, not raw.** A control byte either starts a literal
run or repeats one pixel. Worst-case expansion is under one percent, so a
camera frame costs about what the raw pixels cost, while a user interface
screen with large flat areas costs far less.

**The panel is copied, not re-rendered.** Every pixel that reaches the panel
passes through one function in the display transport. Hooking that one place
means the feed covers the canvas, the navigation rail and the camera at
once, and no application has to know the feed exists. The alternative,
asking each screen to draw itself twice, would have put the feature into
every application.

**Only changed rows are sent.** The copy lives in PSRAM (pseudo-static RAM,
the external 8 MiB memory chip) with a dirty span per row: the first and
last changed column. An encoder task sweeps them every 40 ms. A user
interface screen changes little, so it arrives almost at once.

**Text has priority over packets.** The writer task sends log text first and
sends each packet whole. A busy screen therefore never delays the log, and
text never lands in the middle of a packet. The log is the thing you cannot
afford to lose; the image is.

**The copy can be switched off while nobody watches.** Copying costs CPU0
time for every pixel drawn — about 6 ms for Face ID's 184x240 camera
preview. An application that both draws a lot and computes a lot calls
`logging::mirror_only_when_watched(true)`. The copy then runs from a refresh
request until the computer stops reading the port for two seconds. Because
the copy is out of date when the next viewer arrives, the application draws
everything again when `logging::mirror_refreshes()` changes. Face ID does
this; without the call the copy is always made.

**Any byte from the computer is a refresh request.** No command language, no
parser on the device. The page sends `R` whenever it opens the port, which
covers a page reload while the board keeps running.

**A panic stops the writer first.** The `custom_pre_backtrace` hook parks the
writer and waits a few milliseconds, so the writer finishes its current
64-byte chunk before esp-backtrace prints. Panic output starts on a clean
line instead of inside a packet.

## Accepted costs

- **A camera screen shows about 2 to 3 frames per second**, limited by the
  port, and can show a tear line: the encoder samples the copy while CPU0
  writes the next frame. Fine for a presentation.
- **CPU0 pays for the copy** on every pixel drawn, unless the application
  switches it off while nobody watches. User interface screens see no
  measurable change.
- **A late viewer first receives the buffered boot log**, then the refresh
  brings the image up to date.
- **Log lines written after a panic are lost**, because the writer stays
  parked. Clean panic output was judged worth more.
- **With no program reading the port**, the writer waits, the text queue
  fills, and further lines are dropped and counted; a warning follows when
  the queue drains. The encoder waits for a free slot. Both recover when a
  reader attaches.

## Where the code is

| Part | Where |
| --- | --- |
| Wire format, run encoder, COBS, CRC, and the decoders the tests use | `crates/core/src/screen.rs` |
| The copy of the panel, the dirty spans, the encoder task | `src/logging/mirror.rs` |
| The text queue, the packet slots, the USB writer and reader | `src/logging/serial.rs` |
| The hook that reports every batch of pixels | `src/capabilities/display/transport.rs` |
| Splitting text from packets on the computer | `tools/autoflash/src/stream.ts` |
| Drawing the packets into a canvas | `tools/autoflash/src/screen.ts` |
