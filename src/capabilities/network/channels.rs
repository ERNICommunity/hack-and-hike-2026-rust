//! Cross-core queues between the application handle (CPU0) and the radio
//! runtime (CPU1).

use core::sync::atomic::{AtomicU32, Ordering};

use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex, channel::Channel, signal::Signal,
};
use serde::Serialize;

use super::{
    message::{IncomingMessage, OutgoingMessage, SendError, serialize_payload},
    protocol::DeviceId,
    state::{Peer, QueueCounters, Snapshot},
};

/// Messages the application may queue before the radio has sent them.
const OUTGOING_QUEUE_LENGTH: usize = 4;
/// Messages the radio may receive before the application reads them.
const INCOMING_QUEUE_LENGTH: usize = 4;

struct Service {
    latest: Signal<CriticalSectionRawMutex, Snapshot>,
    outgoing: Channel<CriticalSectionRawMutex, OutgoingMessage, OUTGOING_QUEUE_LENGTH>,
    incoming: Channel<CriticalSectionRawMutex, IncomingMessage, INCOMING_QUEUE_LENGTH>,
    tx_queue_full: AtomicU32,
    rx_queue_full: AtomicU32,
}

static SERVICE: Service = Service {
    latest: Signal::new(),
    outgoing: Channel::new(),
    incoming: Channel::new(),
    tx_queue_full: AtomicU32::new(0),
    rx_queue_full: AtomicU32::new(0),
};

/// Application handle for ESP-NOW messaging.
pub struct Network {
    service: &'static Service,
    snapshot: Option<Snapshot>,
}

impl Network {
    fn refresh(&mut self) {
        if let Some(snapshot) = self.service.latest.try_take() {
            self.snapshot = Some(snapshot);
        }
    }

    /// The newest peer table and counters, once the radio has published one.
    pub fn snapshot(&mut self) -> Option<&Snapshot> {
        self.refresh();
        self.snapshot.as_ref()
    }

    /// The peers currently in range.
    pub fn peers(&mut self) -> impl Iterator<Item = &Peer> {
        self.refresh();
        self.snapshot.iter().flat_map(Snapshot::peers)
    }

    /// The next received message, if any is waiting.
    pub fn receive(&mut self) -> Option<IncomingMessage> {
        self.service.incoming.try_receive().ok()
    }

    /// Queue `value` for every peer in range. Returns as soon as the message
    /// is queued; the radio sends it shortly after.
    pub fn broadcast<T: Serialize>(&mut self, value: &T) -> Result<(), SendError> {
        self.enqueue(None, value)
    }

    /// Queue `value` for one peer. Fails with [`SendError::UnknownPeer`] when
    /// that device is not in the current peer table.
    pub fn send_to<T: Serialize>(&mut self, peer: DeviceId, value: &T) -> Result<(), SendError> {
        if !self.peers().any(|known| known.id == peer) {
            return Err(SendError::UnknownPeer);
        }
        self.enqueue(Some(peer), value)
    }

    fn enqueue<T: Serialize>(
        &mut self,
        recipient: Option<DeviceId>,
        value: &T,
    ) -> Result<(), SendError> {
        let payload = serialize_payload(value)?;
        self.service
            .outgoing
            .try_send(OutgoingMessage { recipient, payload })
            .map_err(|_| {
                self.service.tx_queue_full.fetch_add(1, Ordering::Relaxed);
                SendError::QueueFull
            })
    }
}

#[derive(Clone, Copy)]
pub(crate) struct Runtime {
    service: &'static Service,
}

impl Runtime {
    pub(super) fn publish(self, snapshot: Snapshot) {
        self.service.latest.signal(snapshot);
    }

    pub(super) fn queue_counters(self) -> QueueCounters {
        QueueCounters {
            tx_queue_full: self.service.tx_queue_full.load(Ordering::Relaxed),
            rx_queue_full: self.service.rx_queue_full.load(Ordering::Relaxed),
        }
    }

    /// Wait for the next message the application wants sent.
    pub(super) async fn next_outgoing(self) -> OutgoingMessage {
        self.service.outgoing.receive().await
    }

    /// Hand a received message to the application, dropping it when the
    /// application has fallen behind.
    pub(super) fn deliver(self, message: IncomingMessage) {
        if self.service.incoming.try_send(message).is_err() {
            self.service.rx_queue_full.fetch_add(1, Ordering::Relaxed);
        }
    }
}

pub(crate) struct Endpoints {
    pub(crate) handle: Network,
    pub(crate) runtime: Runtime,
}

pub(crate) fn endpoints() -> Endpoints {
    Endpoints {
        handle: Network {
            service: &SERVICE,
            snapshot: None,
        },
        runtime: Runtime { service: &SERVICE },
    }
}
