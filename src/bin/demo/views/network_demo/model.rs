//! Periodic ping/pong behavior and presentation state for the network demo.

use embassy_time::{Duration, Instant};

use hack_and_hike::capabilities::network;

use super::DemoMessage;

const NETWORK_UPDATE: Duration = Duration::from_millis(50);
const PING_PERIOD: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug)]
pub(crate) struct DisplayState {
    pub(crate) snapshot: Option<network::Snapshot>,
    pub(crate) pings_sent: u32,
    pub(crate) pings_received: u32,
    pub(crate) pongs_received: u32,
    pub(crate) send_errors: u32,
    pub(crate) decode_errors: u32,
}

pub(crate) struct Model {
    network: network::Network,
    display: DisplayState,
    next_ping_sequence: u32,
    last_update: Instant,
    last_ping: Instant,
    dirty: bool,
}

impl Model {
    pub(crate) fn new(network: network::Network) -> Self {
        let now = Instant::now();
        Self {
            network,
            display: DisplayState {
                snapshot: None,
                pings_sent: 0,
                pings_received: 0,
                pongs_received: 0,
                send_errors: 0,
                decode_errors: 0,
            },
            next_ping_sequence: 1,
            last_update: now,
            last_ping: now,
            dirty: true,
        }
    }

    pub(crate) fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    pub(crate) fn update_if_due(&mut self, now: Instant) {
        if now - self.last_update < NETWORK_UPDATE {
            return;
        }
        self.last_update = now;

        if self.network.refresh() {
            self.display.snapshot = self.network.snapshot().copied();
            self.dirty = true;
        }

        while let Some(message) = self.network.receive() {
            match message.decode::<DemoMessage>() {
                Ok(DemoMessage::Ping { sequence }) => {
                    self.display.pings_received = self.display.pings_received.wrapping_add(1);
                    if self
                        .network
                        .send(Some(message.sender), &DemoMessage::Pong { sequence })
                        .is_err()
                    {
                        self.display.send_errors = self.display.send_errors.wrapping_add(1);
                    }
                    self.dirty = true;
                }
                Ok(DemoMessage::Pong { .. }) => {
                    self.display.pongs_received = self.display.pongs_received.wrapping_add(1);
                    self.dirty = true;
                }
                Err(_) => {
                    self.display.decode_errors = self.display.decode_errors.wrapping_add(1);
                    self.dirty = true;
                }
            }
        }

        if now - self.last_ping >= PING_PERIOD {
            self.last_ping = now;
            let sequence = self.next_ping_sequence;
            match self.network.send(None, &DemoMessage::Ping { sequence }) {
                Ok(()) => {
                    self.next_ping_sequence = self.next_ping_sequence.wrapping_add(1);
                    self.display.pings_sent = self.display.pings_sent.wrapping_add(1);
                }
                Err(_) => {
                    self.display.send_errors = self.display.send_errors.wrapping_add(1);
                }
            }
            self.dirty = true;
        }
    }

    pub(crate) fn take_display(&mut self) -> Option<DisplayState> {
        if !self.dirty {
            return None;
        }
        self.dirty = false;
        Some(self.display)
    }
}
