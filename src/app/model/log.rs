//! Log presentation model.

use embassy_time::{Duration, Instant};

use crate::{data_plane, logger};

const LOG_REFRESH: Duration = Duration::from_millis(100);

pub(super) struct Model {
    input: logger::Input,
    bytes: data_plane::FixedPsramBuffer<u8>,
    len: usize,
    revision: u32,
    last_check: Instant,
    dirty: bool,
}

impl Model {
    pub(super) fn new(input: logger::Input) -> Self {
        Self {
            input,
            bytes: data_plane::FixedPsramBuffer::filled(logger::HISTORY_BYTES, 0),
            len: 0,
            revision: u32::MAX,
            last_check: Instant::now(),
            dirty: true,
        }
    }

    pub(super) fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    pub(super) fn update_if_due(&mut self, now: Instant) {
        if now - self.last_check < LOG_REFRESH {
            return;
        }
        self.last_check = now;
        self.refresh();
    }

    pub(super) fn refresh(&mut self) {
        if self.input.revision() == self.revision {
            return;
        }
        let Some((logs, revision)) = self.input.snapshot(self.bytes.as_mut_slice()) else {
            return;
        };
        self.len = logs.len();
        self.revision = revision;
        self.dirty = true;
    }

    pub(super) fn with_text<R>(&mut self, render: impl FnOnce(&str) -> R) -> Option<R> {
        if !self.dirty {
            return None;
        }
        self.dirty = false;
        let bytes = self.bytes.as_slice().get(..self.len)?;
        let text = core::str::from_utf8(bytes).ok()?;
        Some(render(text))
    }
}
