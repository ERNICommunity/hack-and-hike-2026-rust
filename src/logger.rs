use core::cell::RefCell;
use core::fmt::Write;

use critical_section::Mutex;
use log::{LevelFilter, Metadata, Record};

use crate::data_plane::PsramByteRing;

pub const HISTORY_BYTES: usize = 32 * 1024;

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

    ::log::info!("PSRAM log history enabled: {} KiB", HISTORY_BYTES / 1024);
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

pub fn snapshot<'a>(out: &'a mut [u8]) -> (&'a str, u32) {
    let (len, revision) = critical_section::with(|cs| {
        let store = LOG_STORE.borrow(cs).borrow();
        let Some(store) = store.as_ref() else {
            return (0, 0);
        };

        (store.history.copy_to(out), store.revision)
    });

    (core::str::from_utf8(&out[..len]).unwrap_or(""), revision)
}
