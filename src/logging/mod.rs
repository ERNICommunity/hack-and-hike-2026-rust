//! Logging to the serial port, with a history that applications can show.
//!
//! Use the `log` macros anywhere, on either CPU core:
//!
//! ```ignore
//! log::info!("button pressed at {}", point.x);
//! log::warn!("send failed: {error}");
//! ```
//!
//! Every record goes to the USB serial port. A record longer than 512 bytes
//! is cut there.
//!
//! The same USB port carries the live screen feed: a copy of the panel that
//! the autoflash page shows next to the log. The log lines stay plain text,
//! so any serial terminal still shows them. The screen packets start and end
//! with a `0x00` byte, which never occurs in the text. See
//! [`hack_and_hike_core::screen`] for the format. The feed needs no
//! application code: the display reports every pixel it sends. An
//! application that draws a lot can switch the copy of the panel off while
//! nobody watches the feed: see [`mirror_only_when_watched`].
//!
//! - `serial`: the text queue, the packet channel and the USB tasks.
//! - `mirror`: the copy of the panel in PSRAM and the packet encoder.
//!
//! When the text queue is full, because no computer reads the port, new
//! lines are dropped and counted. A warning with the count follows.
//!
//! The newest [`LINES`] records are also kept in PSRAM, so an application can
//! show them on the screen with the [`LogHistory`] handle. There, a record
//! longer than [`LINE_BYTES`] bytes is cut and ends in `...`. The first
//! messages of [`Board::init`](crate::Board::init) come before the history
//! exists, so they are only on the serial port.
//!
//! Records at `debug` and `trace` level are not logged at all.

pub(crate) mod mirror;
mod serial;

use core::{cell::RefCell, fmt::Write as _, sync::atomic::Ordering};

use arrayvec::ArrayString;
use critical_section::Mutex;
use embassy_executor::Spawner;
use esp_alloc::HEAP;
use esp_hal::{delay::Delay, peripherals::USB_DEVICE, usb::usb_serial_jtag::UsbSerialJtag};
use log::{LevelFilter, Metadata, Record};

use hack_and_hike_core::lines::LineHistory;

use crate::board::psram;

pub use hack_and_hike_core::lines::{LINE_BYTES, LINES, Line};
pub use mirror::{only_when_watched as mirror_only_when_watched, refreshes as mirror_refreshes};

/// Longest record printed in full on the serial port.
const RECORD_BYTES: usize = 512;
/// The line buffer: a record and its line end.
type RecordLine = ArrayString<{ RECORD_BYTES + 1 }>;
/// How long the panic hook waits for the USB writer to finish its current
/// chunk and stop.
const PANIC_SETTLE_MS: u32 = 5;

/// The history, shared by both CPU cores. The logger starts before PSRAM
/// is ready. So the history is added later, and it is `None` until then.
static HISTORY: Mutex<RefCell<Option<&'static mut LineHistory>>> = Mutex::new(RefCell::new(None));

/// Run `f` on the history inside a critical section. A critical section
/// stops the other core and interrupts from using the history at the same
/// time. Returns `None` before the history exists.
fn with_history<R>(f: impl FnOnce(&mut LineHistory) -> R) -> Option<R> {
    critical_section::with(|cs| HISTORY.borrow(cs).borrow_mut().as_deref_mut().map(f))
}

/// Application handle for the log history.
///
/// The history holds the newest [`LINES`] log lines. Each line is at most
/// [`LINE_BYTES`] bytes long.
///
/// ```ignore
/// let mut lines = [Line::new(); 16];
/// if history.revision() != shown_revision {
///     let count = history.newest(&mut lines);
///     for line in &lines[..count] { /* draw line */ }
/// }
/// ```
pub struct LogHistory {
    /// Prevents construction outside this module.
    _private: (),
}

impl LogHistory {
    /// A number that increases by one with every logged record. Compare it
    /// with the value you saw last, and skip the redraw when it is the same.
    pub fn revision(&self) -> u32 {
        with_history(|history| history.revision()).unwrap_or(0)
    }

    /// Copy the newest lines into `out`, as many as fit, oldest first.
    /// Return the number of lines copied.
    pub fn newest(&self, out: &mut [Line]) -> usize {
        with_history(|history| history.newest(out)).unwrap_or(0)
    }
}

/// The `log` backend: queues each record for the serial port and keeps it
/// in the history.
struct Logger;

impl log::Log for Logger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }

        // For a record longer than the buffer, `Cut` keeps the start and
        // returns an error. The error only stops `write!`, so we ignore it.
        let mut line = RecordLine::new();
        let _ = write!(Cut(&mut line), "[{}] {}", record.level(), record.args());

        with_history(|history| history.push(&line));
        // `Cut` leaves room for this one byte.
        line.push('\n');
        serial::push_text(&line);
    }

    fn flush(&self) {}
}

/// A writer that cuts long records.
///
/// `write!` sends the text in pieces. When a piece does not fit completely,
/// `ArrayString` alone would drop the whole piece. `Cut` keeps the part that
/// fits in [`RECORD_BYTES`], ends on a whole UTF-8 character, and then
/// returns an error. The last byte of the buffer stays free for the line end.
struct Cut<'a>(&'a mut RecordLine);

impl core::fmt::Write for Cut<'_> {
    fn write_str(&mut self, piece: &str) -> core::fmt::Result {
        let room = RECORD_BYTES - self.0.len();
        if piece.len() <= room {
            self.0.push_str(piece);
            return Ok(());
        }
        let mut keep = room;
        while !piece.is_char_boundary(keep) {
            keep -= 1;
        }
        self.0.push_str(&piece[..keep]);
        Err(core::fmt::Error)
    }
}

/// The one logger instance. `log::set_logger` needs a `&'static` reference,
/// so it is a static and not a local value.
static LOGGER: Logger = Logger;

/// Create the text queue and install the logger. Records less important
/// than `level` are dropped.
///
/// # Panics
///
/// When a logger is already installed, or the internal heap has no room for
/// the 8 KiB text queue.
pub(crate) fn init(level: LevelFilter) {
    serial::create_text_queue();
    log::set_logger(&LOGGER)
        .map(|()| log::set_max_level(level))
        .expect("the logger is initialized once");
}

/// Start the USB tasks of the log and the screen feed on the calling core:
/// the writer, the reader for refresh requests, and the screen encoder.
/// The async USB driver handles its interrupt on the core that creates it,
/// so CPU1 calls this function.
///
/// # Panics
///
/// When it is called a second time, or PSRAM has no room for the packet
/// slots.
pub(crate) fn spawn(spawner: &Spawner, usb: USB_DEVICE<'static>) {
    let (rx, tx) = UsbSerialJtag::new(usb).into_async().split();
    let (sender, receiver) = serial::packet_channel();
    let queue = serial::text_queue().expect("logging::init created the text queue");
    spawner
        .spawn(serial::writer_task(tx, queue, receiver).expect("USB writer task already spawned"));
    spawner.spawn(serial::reader_task(rx).expect("USB reader task already spawned"));
    spawner.spawn(mirror::encoder_task(sender).expect("screen encoder task already spawned"));
}

/// Called by the panic handler of `esp-backtrace` before it prints.
///
/// It stops the USB writer, which then parks before its next 64-byte chunk.
/// The short wait lets the writer finish the chunk it is sending. Then the
/// panic message and the backtrace have the port to themselves, with no
/// half-sent packet before them. Log lines after the panic are lost.
#[unsafe(no_mangle)]
fn custom_pre_backtrace() {
    serial::PANICKED.store(true, Ordering::Release);
    Delay::new().delay_millis(PANIC_SETTLE_MS);
}

/// Start keeping a history of log records in PSRAM, and return the handle
/// for it.
///
/// # Panics
///
/// When PSRAM is not enabled yet, or has no room for the history.
pub(crate) fn enable_history() -> LogHistory {
    let history = psram::leaked_value(LineHistory::new);
    critical_section::with(|cs| *HISTORY.borrow(cs).borrow_mut() = Some(history));
    log::info!("Log history enabled: {LINES} lines of up to {LINE_BYTES} bytes");
    LogHistory { _private: () }
}

/// Log how much of the internal heap and of PSRAM is in use now, and the
/// highest use so far.
///
/// `label` is part of the log line, so you can tell reports apart. The
/// report helps to find the cause of an allocation failure.
pub fn report_memory(label: &str) {
    let internal = HEAP.stats();
    let external = psram::heap().stats();
    log::info!(
        "MEM [{}] internal={}/{} KiB (peak {} KiB) | psram={}/{} KiB (peak {} KiB)",
        label,
        internal.current_usage / 1024,
        internal.size / 1024,
        internal.max_usage / 1024,
        external.current_usage / 1024,
        external.size / 1024,
        external.max_usage / 1024,
    );
}
