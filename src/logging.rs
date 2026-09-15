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
//! The newest [`LINES`] records are also kept in PSRAM, so an application can
//! show them on the screen with the [`LogHistory`] handle. There, a record
//! longer than [`LINE_BYTES`] bytes is cut and ends in `...`. The first
//! messages of [`Board::init`](crate::Board::init) come before the history
//! exists, so they are only on the serial port.
//!
//! Records at `debug` and `trace` level are not logged at all.

use core::{cell::RefCell, fmt::Write as _};

use arrayvec::ArrayString;
use critical_section::Mutex;
use esp_alloc::HEAP;
use log::{LevelFilter, Metadata, Record};

use hack_and_hike_core::lines::LineHistory;

use crate::board::psram;

pub use hack_and_hike_core::lines::{LINE_BYTES, LINES, Line};

/// Longest record printed in full on the serial port.
const RECORD_BYTES: usize = 512;

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

/// The `log` backend: prints each record and keeps it in the history.
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
        let mut line = ArrayString::<RECORD_BYTES>::new();
        let _ = write!(Cut(&mut line), "[{}] {}", record.level(), record.args());

        esp_println::println!("{line}");
        with_history(|history| history.push(&line));
    }

    fn flush(&self) {}
}

/// A writer that cuts long records.
///
/// `write!` sends the text in pieces. When a piece does not fit completely,
/// `ArrayString` alone would drop the whole piece. `Cut` keeps the part that
/// fits, ends on a whole UTF-8 character, and then returns an error.
struct Cut<'a>(&'a mut ArrayString<RECORD_BYTES>);

impl core::fmt::Write for Cut<'_> {
    fn write_str(&mut self, piece: &str) -> core::fmt::Result {
        if self.0.try_push_str(piece).is_ok() {
            return Ok(());
        }
        let mut keep = self.0.remaining_capacity();
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

/// Install the logger. Records less important than `level` are dropped.
///
/// # Panics
///
/// When a logger is already installed.
pub(crate) fn init(level: LevelFilter) {
    log::set_logger(&LOGGER)
        .map(|()| log::set_max_level(level))
        .expect("the logger is initialized once");
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
