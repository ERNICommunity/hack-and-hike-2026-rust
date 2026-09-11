//! Log presentation model.

use embassy_time::{Duration, Instant};

use crate::support::logging;

const LOG_REFRESH: Duration = Duration::from_millis(100);

pub(super) struct Model {
    input: logging::Input,
    history: logging::HistoryBuffer,
    last_check: Instant,
    dirty: bool,
}

impl Model {
    pub(super) fn new(input: logging::Input) -> Self {
        Self {
            input,
            history: logging::HistoryBuffer::new(),
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
        if self.history.refresh(&mut self.input) {
            self.dirty = true;
        }
    }

    pub(super) fn with_text<R>(&mut self, render: impl FnOnce(&str) -> R) -> Option<R> {
        if !self.dirty {
            return None;
        }
        self.dirty = false;
        Some(render(self.history.text()?))
    }
}
