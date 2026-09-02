use core::cell::RefCell;
use core::fmt::Write;

use critical_section::Mutex;
use log::{LevelFilter, Metadata, Record};

pub const SNAPSHOT_BYTES: usize = 2048;

struct LogStore {
    buffer: [u8; SNAPSHOT_BYTES],
    start: usize,
    len: usize,
    revision: u32,
}

impl LogStore {
    const fn new() -> Self {
        Self {
            buffer: [0; SNAPSHOT_BYTES],
            start: 0,
            len: 0,
            revision: 0,
        }
    }

    fn clear(&mut self) {
        self.start = 0;
        self.len = 0;
    }

    /// Drop the oldest complete line from the front of the ring.
    fn drop_oldest_line(&mut self) {
        if self.len == 0 {
            return;
        }

        let mut dropped = 0usize;
        while dropped < self.len {
            let index = (self.start + dropped) % SNAPSHOT_BYTES;
            dropped += 1;
            if self.buffer[index] == b'\n' {
                break;
            }
        }

        self.start = (self.start + dropped) % SNAPSHOT_BYTES;
        self.len -= dropped;
        if self.len == 0 {
            self.start = 0;
        }
    }

    fn make_room(&mut self, needed: usize) -> bool {
        if needed > SNAPSHOT_BYTES {
            self.clear();
            return false;
        }

        while self.len + needed > SNAPSHOT_BYTES {
            self.drop_oldest_line();
        }

        true
    }

    /// Append a complete log line. Storage order is oldest -> newest, which is
    /// also the order expected by the Slint log model.
    fn push_back(&mut self, bytes: &[u8]) {
        if bytes.is_empty() || !self.make_room(bytes.len()) {
            return;
        }

        let end = (self.start + self.len) % SNAPSHOT_BYTES;
        let first_len = bytes.len().min(SNAPSHOT_BYTES - end);
        self.buffer[end..end + first_len].copy_from_slice(&bytes[..first_len]);

        let remaining = bytes.len() - first_len;
        if remaining != 0 {
            self.buffer[..remaining].copy_from_slice(&bytes[first_len..]);
        }

        self.len += bytes.len();
        self.revision = self.revision.wrapping_add(1);
    }

    fn copy_to(&self, out: &mut [u8; SNAPSHOT_BYTES]) -> usize {
        let first_len = self.len.min(SNAPSHOT_BYTES - self.start);
        out[..first_len].copy_from_slice(&self.buffer[self.start..self.start + first_len]);

        let remaining = self.len - first_len;
        if remaining != 0 {
            out[first_len..first_len + remaining].copy_from_slice(&self.buffer[..remaining]);
        }

        self.len
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

        // Fixed-size formatting keeps logging allocation-free.
        let mut line = arrayvec::ArrayString::<256>::new();
        let _ = writeln!(line, "[{}] {}", record.level(), record.args());

        // Serial output remains immediate; the UI receives a bounded snapshot.
        esp_println::print!("{}\r\n", line.trim_end_matches('\n'));

        critical_section::with(|cs| {
            LOG_STORE.borrow(cs).borrow_mut().push_back(line.as_bytes());
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

/// Return the current log-store revision using only a very short critical
/// section. UI code uses this to avoid copying the 2 KiB snapshot unnecessarily.
pub fn revision() -> u32 {
    critical_section::with(|cs| LOG_STORE.borrow(cs).borrow().revision)
}

/// Copy a consistent, contiguous oldest-to-newest snapshot into caller-owned
/// memory, then release the critical section before any Slint work happens.
pub fn snapshot<'a>(out: &'a mut [u8; SNAPSHOT_BYTES]) -> (&'a str, u32) {
    let (len, revision) = critical_section::with(|cs| {
        let store = LOG_STORE.borrow(cs).borrow();
        (store.copy_to(out), store.revision)
    });

    (core::str::from_utf8(&out[..len]).unwrap_or(""), revision)
}
