use core::cell::RefCell;
use core::fmt::Write;
use critical_section::Mutex;
use log::{LevelFilter, Metadata, Record};

const MAX_LOG_SIZE: usize = 1024;

struct LogStore {
    buffer: [u8; MAX_LOG_SIZE],
    head: usize,
    len: usize,
}

impl LogStore {
    const fn new() -> Self {
        Self {
            buffer: [0u8; MAX_LOG_SIZE],
            head: 0,
            len: 0,
        }
    }

    /// Evicts the oldest line (all bytes from `head` up to and including the next `\n`)
    fn pop_oldest_line(&mut self) {
        if self.len == 0 {
            return;
        }

        let mut offset = 0;
        while offset < self.len {
            let idx = (self.head + offset) % MAX_LOG_SIZE;
            if self.buffer[idx] == b'\n' {
                let drop_count = offset + 1;
                self.head = (self.head + drop_count) % MAX_LOG_SIZE;
                self.len -= drop_count;
                return;
            }
            offset += 1;
        }

        // If no newline exists (e.g. a single message filled the entire buffer), clear it
        self.head = 0;
        self.len = 0;
    }

    /// Removes old lines from the beginning of the buffer until `needed` bytes can fit
    fn ensure_capacity(&mut self, needed: usize) {
        if needed > MAX_LOG_SIZE {
            self.head = 0;
            self.len = 0;
            return;
        }

        while self.len + needed > MAX_LOG_SIZE && self.len > 0 {
            self.pop_oldest_line();
        }
    }

    /// Appends a byte slice to the circular buffer
    fn push_bytes(&mut self, bytes: &[u8]) {
        for &b in bytes {
            let idx = (self.head + self.len) % MAX_LOG_SIZE;
            self.buffer[idx] = b;
            self.len += 1;
        }
    }

    /// Returns a contiguous `&str` of the accumulated logs for Slint.
    ///
    /// If the data is wrapped around the end of the circular buffer, this rearranges
    /// the buffer in-place so that `head` becomes `0`
    fn as_str(&mut self) -> &str {
        if self.len == 0 {
            return "";
        }

        // If the data is wrapped around the end of the array, rotate in-place to linearize it
        if self.head + self.len > MAX_LOG_SIZE {
            self.buffer.rotate_left(self.head);
            self.head = 0;
        }

        core::str::from_utf8(&self.buffer[self.head..self.head + self.len]).unwrap_or("")
    }
}

static LOG_STORE: Mutex<RefCell<LogStore>> = Mutex::new(RefCell::new(LogStore::new()));

pub struct Logger;

impl log::Log for Logger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }

        // Format once into a small temporary 256-byte stack buffer
        let mut line_buf = arrayvec::ArrayString::<256>::new();
        let _ = writeln!(line_buf, "[{:<5}] {}", record.level(), record.args());

        // 1. Output to Serial Terminal
        esp_println::print!("{}\r\n", line_buf.trim_end_matches('\n'));

        // 2. Append to Ring Buffer directly
        critical_section::with(|cs| {
            let mut store = LOG_STORE.borrow(cs).borrow_mut();
            store.ensure_capacity(line_buf.len());
            store.push_bytes(line_buf.as_bytes());
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

/// Helper function for Slint to read the accumulated log buffer
pub fn with_logs(f: impl FnOnce(&str)) {
    critical_section::with(|cs| {
        let mut store = LOG_STORE.borrow(cs).borrow_mut();
        let logs = store.as_str();
        f(logs);
    });
}
