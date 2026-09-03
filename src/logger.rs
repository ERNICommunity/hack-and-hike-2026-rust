use core::cell::RefCell;
use core::fmt::Write;

use critical_section::Mutex;
use log::{LevelFilter, Metadata, Record};

use crate::data_plane::PsramByteRing;

/// Maximum number of rows retained by the on-device log model.
pub const MAX_LOG_ROWS: usize = 64;

/// PSRAM byte budget per retained row. Log lines may be longer or shorter than
/// this; this constant only sizes the byte ring from the row-count policy.
const LOG_BYTES_PER_ROW_BUDGET: usize = 64;

pub const HISTORY_BYTES: usize = MAX_LOG_ROWS * LOG_BYTES_PER_ROW_BUDGET;
const SNAPSHOT_CHUNK_BYTES: usize = 512;

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

// The lock/control object remains internal. Only the byte-ring allocation is
// external PSRAM data.
static LOG_STORE: Mutex<RefCell<Option<LogStore>>> = Mutex::new(RefCell::new(None));

pub struct Logger;

impl log::Log for Logger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }

        let mut line = arrayvec::ArrayString::<256>::new();
        let _ = writeln!(line, "[{}] {}", record.level(), record.args());

        esp_println::print!("{}\r\n", line.trim_end_matches('\n'));

        critical_section::with(|cs| {
            if let Some(store) = LOG_STORE.borrow(cs).borrow_mut().as_mut() {
                store.push_back(line.as_bytes());
            }
        });
    }

    fn flush(&self) {}
}

static LOGGER: Logger = Logger;

pub fn init(level: LevelFilter) {
    log::set_logger(&LOGGER)
        .map(|()| log::set_max_level(level))
        .expect("Failed to initialize logger");
}

pub fn enable_psram_history() {
    let store = LogStore::new();

    critical_section::with(|cs| {
        *LOG_STORE.borrow(cs).borrow_mut() = Some(store);
    });

    ::log::info!(
        "PSRAM log history enabled: {} rows, {} KiB",
        MAX_LOG_ROWS,
        HISTORY_BYTES / 1024
    );
}

pub fn revision() -> u32 {
    critical_section::with(|cs| {
        LOG_STORE
            .borrow(cs)
            .borrow()
            .as_ref()
            .map_or(0, |store| store.revision)
    })
}

/// Copy one consistent log-history revision into `out` using short critical
/// sections. If a writer changes the ring while the snapshot is in progress,
/// return `None` and let the UI retry on its next refresh tick.
pub fn snapshot<'a>(out: &'a mut [u8]) -> Option<(&'a str, u32)> {
    let (len, revision) = critical_section::with(|cs| {
        let store = LOG_STORE.borrow(cs).borrow();
        let store = store.as_ref()?;
        Some((store.history.len().min(out.len()), store.revision))
    })?;

    let mut offset = 0usize;
    while offset < len {
        let end = (offset + SNAPSHOT_CHUNK_BYTES).min(len);
        let copied = critical_section::with(|cs| {
            let store = LOG_STORE.borrow(cs).borrow();
            let Some(store) = store.as_ref() else {
                return 0;
            };

            if store.revision != revision {
                return 0;
            }

            store
                .history
                .copy_range_to(offset, &mut out[offset..end])
        });

        if copied != end - offset {
            return None;
        }

        offset = end;
    }

    if self::revision() != revision {
        return None;
    }

    let text = core::str::from_utf8(&out[..len]).ok()?;
    Some((text, revision))
}
