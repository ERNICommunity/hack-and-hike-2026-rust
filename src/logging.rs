//! Logging to the serial port, with a history that applications can show.
//!
//! Use the `log` macros anywhere, on either core:
//!
//! ```ignore
//! log::info!("button pressed at {}", point.x);
//! log::warn!("send failed: {error}");
//! ```
//!
//! Every record goes to the USB serial port, and its first [`LINE_BYTES`]
//! bytes are kept in a [`LineHistory`] in PSRAM that the [`LogHistory`]
//! handle reads. Records at `debug` and `trace` level are filtered out.

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

/// The history, shared by every core that logs. The logger exists before
/// PSRAM does, so the history is attached later and is `None` until then.
static HISTORY: Mutex<RefCell<Option<&'static mut LineHistory>>> = Mutex::new(RefCell::new(None));

/// Run `f` on the history inside a critical section; `None` before the
/// history exists.
fn with_history<R>(f: impl FnOnce(&mut LineHistory) -> R) -> Option<R> {
    critical_section::with(|cs| HISTORY.borrow(cs).borrow_mut().as_deref_mut().map(f))
}

/// Application handle for the log history.
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
    /// Increments with every logged record; compare it to skip redraws.
    pub fn revision(&self) -> u32 {
        with_history(|history| history.revision()).unwrap_or(0)
    }

    /// Copy the newest lines into `out`, oldest first, as many as fit.
    /// Returns how many were copied.
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

        // A record longer than the buffer is cut; `write!` then reports an
        // error that is deliberately ignored.
        let mut line = ArrayString::<RECORD_BYTES>::new();
        let _ = write!(line, "[{}] {}", record.level(), record.args());

        esp_println::println!("{line}");
        with_history(|history| history.push(&line));
    }

    fn flush(&self) {}
}

/// The one logger instance. `log::set_logger` needs a `&'static` reference,
/// so it is a static rather than a local value.
static LOGGER: Logger = Logger;

/// Install the logger, dropping records below `level`.
pub(crate) fn init(level: LevelFilter) {
    log::set_logger(&LOGGER)
        .map(|()| log::set_max_level(level))
        .expect("the logger is initialized once");
}

/// Start keeping a history of log records in PSRAM.
pub(crate) fn enable_history() -> LogHistory {
    let history = psram::leaked_value(LineHistory::new);
    critical_section::with(|cs| *HISTORY.borrow(cs).borrow_mut() = Some(history));
    log::info!("Log history enabled: {LINES} lines of up to {LINE_BYTES} bytes");
    LogHistory { _private: () }
}

/// Log how much internal heap and PSRAM is in use, with `label` to tell
/// reports apart. Handy when chasing an allocation failure.
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
