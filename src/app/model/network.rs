//! Network presentation model.

use embassy_time::{Duration, Instant};

use crate::capabilities::network;

const NETWORK_UPDATE: Duration = Duration::from_millis(200);

pub(super) struct Model {
    input: network::Input,
    display: Option<network::Snapshot>,
    last_revision: u32,
    last_update: Instant,
    dirty: bool,
}

impl Model {
    pub(super) fn new(input: network::Input) -> Self {
        Self {
            input,
            display: None,
            last_revision: u32::MAX,
            last_update: Instant::now(),
            dirty: false,
        }
    }

    pub(super) fn mark_dirty(&mut self) {
        if self.display.is_some() {
            self.dirty = true;
        }
    }

    pub(super) fn update_if_due(&mut self, now: Instant) {
        if now - self.last_update < NETWORK_UPDATE {
            return;
        }
        self.last_update = now;
        let Some(snapshot) = self.input.take_latest() else {
            return;
        };
        if snapshot.revision == self.last_revision {
            return;
        }
        self.last_revision = snapshot.revision;
        self.display = Some(snapshot);
        self.dirty = true;
    }

    pub(super) fn take_display(&mut self) -> Option<network::Snapshot> {
        if !self.dirty {
            return None;
        }
        self.dirty = false;
        self.display
    }
}
