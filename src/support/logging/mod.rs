use core::cell::RefCell;
use core::fmt::Write;

use critical_section::Mutex;
use log::{LevelFilter, Metadata, Record};

use crate::support::memory::storage::PsramByteRing;

/// Maximum number of rows retained by the on-device log model.
pub(crate) const MAX_LOG_ROWS: usize = 64;

/// Fixed stack budget for formatting one log record.
///
/// The runtime memory diagnostic is intentionally verbose and exceeded the old
/// 256-byte buffer. A larger fixed buffer keeps that record intact without any
/// heap allocation on either CPU.
const LOG_RECORD_BYTES: usize = 512;

/// PSRAM byte budget per retained row. Log lines may be longer or shorter than
/// this; this constant only sizes the byte ring from the row-count policy.
const LOG_BYTES_PER_ROW_BUDGET: usize = 256;

pub(crate) const HISTORY_BYTES: usize = MAX_LOG_ROWS * LOG_BYTES_PER_ROW_BUDGET;
const SNAPSHOT_CHUNK_BYTES: usize = 512;
const TRUNCATION_SUFFIX: &str = "...\n";

struct LogStore {
    history: PsramByteRing,
    revision: u32,
}

impl LogStore {
    fn new() -> Self {
        Self {
            history: PsramByteRing::new(HISTORY_BYTES),
            revision: 0,
        }
    }

    fn push_back(&mut self, bytes: &[u8]) {
        if self.history.push_line(bytes) {
            self.revision = self.revision.wrapping_add(1);
        }
    }
}

type Store = Mutex<RefCell<Option<LogStore>>>;

// The logger must exist before PSRAM history is enabled, so its service storage
// starts empty and is populated during bootstrap. The static lifetime is an
// implementation detail; application code receives only `Input`.
struct Service {
    store: Store,
}

impl Service {
    const fn new() -> Self {
        Self {
            store: Mutex::new(RefCell::new(None)),
        }
    }
}

static SERVICE: Service = Service::new();

pub(crate) struct Input {
    service: &'static Service,
}

impl Input {
    pub(crate) fn revision(&self) -> u32 {
        critical_section::with(|cs| {
            self.service
                .store
                .borrow(cs)
                .borrow()
                .as_ref()
                .map_or(0, |store| store.revision)
        })
    }

    /// Copy one consistent log-history revision into `out` using short critical
    /// sections. If a writer changes the ring while the snapshot is in progress,
    /// return `None` and let the UI retry on its next refresh tick.
    pub(crate) fn snapshot<'a>(&mut self, out: &'a mut [u8]) -> Option<(&'a str, u32)> {
        let (len, revision) = critical_section::with(|cs| {
            let store = self.service.store.borrow(cs).borrow();
            let store = store.as_ref()?;
            Some((store.history.len().min(out.len()), store.revision))
        })?;

        let mut offset = 0usize;
        while offset < len {
            let end = (offset + SNAPSHOT_CHUNK_BYTES).min(len);
            let copied = critical_section::with(|cs| {
                let store = self.service.store.borrow(cs).borrow();
                let Some(store) = store.as_ref() else {
                    return 0;
                };

                if store.revision != revision {
                    return 0;
                }

                store.history.copy_range_to(offset, &mut out[offset..end])
            });

            if copied != end - offset {
                return None;
            }

            offset = end;
        }

        if self.revision() != revision {
            return None;
        }

        let text = core::str::from_utf8(&out[..len]).ok()?;
        Some((text, revision))
    }
}

struct Logger;

impl log::Log for Logger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }

        // Keep record framing independent from formatting success. `write!`
        // may leave a full ArrayString when the message is too long; storing
        // that buffer without a trailing newline would make the PSRAM history
        // merge this record with every following one. Always reserve/append a
        // newline, truncating with an explicit marker only when necessary.
        let mut line = arrayvec::ArrayString::<LOG_RECORD_BYTES>::new();
        let formatted = write!(line, "[{}] {}", record.level(), record.args());

        if formatted.is_err() || line.len() == LOG_RECORD_BYTES {
            while line.len() > LOG_RECORD_BYTES - TRUNCATION_SUFFIX.len() {
                let _ = line.pop();
            }
            line.push_str(TRUNCATION_SUFFIX);
        } else {
            line.push('\n');
        }

        esp_println::print!("{}\r\n", line.trim_end_matches('\n'));

        critical_section::with(|cs| {
            if let Some(store) = SERVICE.store.borrow(cs).borrow_mut().as_mut() {
                store.push_back(line.as_bytes());
            }
        });
    }

    fn flush(&self) {}
}

static LOGGER: Logger = Logger;

pub(crate) fn init(level: LevelFilter) {
    log::set_logger(&LOGGER)
        .map(|()| log::set_max_level(level))
        .expect("Failed to initialize logger");
}

pub(crate) fn enable_psram_history() -> Input {
    let store = LogStore::new();

    critical_section::with(|cs| {
        *SERVICE.store.borrow(cs).borrow_mut() = Some(store);
    });

    ::log::info!(
        "PSRAM log history enabled: {} rows, {} KiB",
        MAX_LOG_ROWS,
        HISTORY_BYTES / 1024
    );

    Input { service: &SERVICE }
}
