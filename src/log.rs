use core::cell::RefCell;
use core::fmt::Write;
use critical_section::Mutex;
use log::{LevelFilter, Metadata, Record};

const MAX_LOG_SIZE: usize = 2048;

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

    /// Evicts the oldest line from the tail (end) of the buffer
    fn pop_oldest_line(&mut self) {
        if self.len == 0 {
            return;
        }

        // Search backwards from the tail (skipping the final newline) for the preceding newline
        if self.len > 1 {
            let mut offset = self.len - 2;
            loop {
                let idx = (self.head + offset) % MAX_LOG_SIZE;
                if self.buffer[idx] == b'\n' {
                    let drop_count = self.len - (offset + 1);
                    self.len -= drop_count;
                    return;
                }
                if offset == 0 {
                    break;
                }
                offset -= 1;
            }
        }

        // If no preceding newline exists, clear the buffer
        self.head = 0;
        self.len = 0;
    }

    /// Removes old lines from the tail until `needed` bytes can fit
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

    /// Prepends a byte slice to the head of the circular buffer
    fn push_bytes_front(&mut self, bytes: &[u8]) {
        if bytes.len() > MAX_LOG_SIZE || bytes.is_empty() {
            return;
        }

        // Move head backwards to allocate space for the incoming line
        self.head = (self.head + MAX_LOG_SIZE - bytes.len()) % MAX_LOG_SIZE;
        self.len += bytes.len();

        for (i, &b) in bytes.iter().enumerate() {
            let idx = (self.head + i) % MAX_LOG_SIZE;
            self.buffer[idx] = b;
        }
    }

    /// Returns a contiguous `&str` of the accumulated logs for Slint.
    fn as_str(&mut self) -> &str {
        if self.len == 0 {
            return "";
        }

        // Rotate in-place to linearize wrapped memory so head starts at 0
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

        let mut line_buf = arrayvec::ArrayString::<256>::new();
        let _ = writeln!(line_buf, "[{}] {}", record.level(), record.args());

        // 1. Output to Serial Terminal
        esp_println::print!("{}\r\n", line_buf.trim_end_matches('\n'));

        // 2. Prepend to Ring Buffer (latest lines at index 0)
        critical_section::with(|cs| {
            let mut store = LOG_STORE.borrow(cs).borrow_mut();
            store.ensure_capacity(line_buf.len());
            store.push_bytes_front(line_buf.as_bytes());
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

pub fn with_logs(f: impl FnOnce(&str)) {
    critical_section::with(|cs| {
        let mut store = LOG_STORE.borrow(cs).borrow_mut();
        let logs = store.as_str();
        f(logs);
    });
}
