//! Bounded cross-core transports for network snapshots and application messages.

use core::sync::atomic::{AtomicU32, Ordering};

use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex,
    mutex::Mutex,
    signal::Signal,
};
use serde::Serialize;
use static_cell::StaticCell;

use super::{
    DeviceId, IncomingMessage, MAX_PAYLOAD, Peer, SendError, Snapshot, message::serialize_payload,
};

const TX_QUEUE_CAPACITY: usize = 4;
const RX_QUEUE_CAPACITY: usize = 4;

type SnapshotSignal = Signal<CriticalSectionRawMutex, Snapshot>;

struct QueuedTx {
    recipient: Option<DeviceId>,
    payload: [u8; MAX_PAYLOAD],
    len: usize,
}

pub(super) struct OutgoingMessage {
    pub(super) recipient: Option<DeviceId>,
    pub(super) payload: [u8; MAX_PAYLOAD],
    pub(super) len: usize,
}

struct TxQueue {
    items: [Option<QueuedTx>; TX_QUEUE_CAPACITY],
    read_index: usize,
    len: usize,
}

impl TxQueue {
    const fn new() -> Self {
        Self {
            items: [const { None }; TX_QUEUE_CAPACITY],
            read_index: 0,
            len: 0,
        }
    }

    fn push(&mut self, recipient: Option<DeviceId>, payload: &[u8]) -> bool {
        if self.len == TX_QUEUE_CAPACITY {
            return false;
        }
        let write_index = (self.read_index + self.len) % TX_QUEUE_CAPACITY;
        let mut bytes = [0u8; MAX_PAYLOAD];
        bytes[..payload.len()].copy_from_slice(payload);
        self.items[write_index] = Some(QueuedTx {
            recipient,
            payload: bytes,
            len: payload.len(),
        });
        self.len += 1;
        true
    }

    fn pop(&mut self) -> Option<OutgoingMessage> {
        if self.len == 0 {
            return None;
        }
        let index = self.read_index;
        let item = self.items[index].take()?;
        self.read_index = (self.read_index + 1) % TX_QUEUE_CAPACITY;
        self.len -= 1;
        Some(OutgoingMessage {
            recipient: item.recipient,
            payload: item.payload,
            len: item.len,
        })
    }
}

struct QueuedRx {
    sender: DeviceId,
    recipient: Option<DeviceId>,
    payload: [u8; MAX_PAYLOAD],
    len: usize,
}

struct RxQueue {
    items: [Option<QueuedRx>; RX_QUEUE_CAPACITY],
    read_index: usize,
    len: usize,
}

impl RxQueue {
    const fn new() -> Self {
        Self {
            items: [const { None }; RX_QUEUE_CAPACITY],
            read_index: 0,
            len: 0,
        }
    }

    fn push(&mut self, sender: DeviceId, recipient: Option<DeviceId>, payload: &[u8]) -> bool {
        if self.len == RX_QUEUE_CAPACITY || payload.len() > MAX_PAYLOAD {
            return false;
        }
        let write_index = (self.read_index + self.len) % RX_QUEUE_CAPACITY;
        let mut bytes = [0u8; MAX_PAYLOAD];
        bytes[..payload.len()].copy_from_slice(payload);
        self.items[write_index] = Some(QueuedRx {
            sender,
            recipient,
            payload: bytes,
            len: payload.len(),
        });
        self.len += 1;
        true
    }

    fn pop(&mut self) -> Option<IncomingMessage> {
        if self.len == 0 {
            return None;
        }
        let index = self.read_index;
        let item = self.items[index].take()?;
        self.read_index = (self.read_index + 1) % RX_QUEUE_CAPACITY;
        self.len -= 1;
        IncomingMessage::from_bytes(item.sender, item.recipient, &item.payload[..item.len])
    }
}

struct Service {
    latest: SnapshotSignal,
    tx: Mutex<CriticalSectionRawMutex, TxQueue>,
    rx: Mutex<CriticalSectionRawMutex, RxQueue>,
    tx_queue_full: AtomicU32,
    rx_queue_full: AtomicU32,
}

impl Service {
    const fn new() -> Self {
        Self {
            latest: Signal::new(),
            tx: Mutex::new(TxQueue::new()),
            rx: Mutex::new(RxQueue::new()),
            tx_queue_full: AtomicU32::new(0),
            rx_queue_full: AtomicU32::new(0),
        }
    }
}

static SERVICE: StaticCell<Service> = StaticCell::new();

#[derive(Clone, Copy)]
pub(crate) struct Runtime {
    service: &'static Service,
}

/// CPU0 application-facing network capability.
pub(crate) struct Network {
    service: &'static Service,
    snapshot: Option<Snapshot>,
}

pub(crate) struct Endpoints {
    pub(crate) runtime: Runtime,
    pub(crate) network: Network,
}

pub(crate) fn init_endpoints() -> Endpoints {
    let service: &'static Service = SERVICE.init(Service::new());
    Endpoints {
        runtime: Runtime { service },
        network: Network {
            service,
            snapshot: None,
        },
    }
}

impl Network {
    /// Refresh the CPU0 peer/diagnostic cache from the newest CPU1 snapshot.
    #[allow(dead_code, reason = "part of the application-facing network diagnostics capability contract")]
    pub(crate) fn refresh(&mut self) -> bool {
        let Some(snapshot) = self.service.latest.try_take() else {
            return false;
        };
        self.snapshot = Some(snapshot);
        true
    }

    #[allow(dead_code, reason = "part of the application-facing network diagnostics capability contract")]
    pub(crate) fn snapshot(&self) -> Option<&Snapshot> {
        self.snapshot.as_ref()
    }

    pub(crate) fn peers(&self) -> impl Iterator<Item = &Peer> {
        self.snapshot.iter().flat_map(|snapshot| snapshot.peers())
    }

    pub(crate) fn receive(&mut self) -> Option<IncomingMessage> {
        let mut queue = self.service.rx.try_lock().ok()?;
        queue.pop()
    }

    pub(crate) fn send<T: Serialize>(
        &mut self,
        recipient: Option<DeviceId>,
        value: &T,
    ) -> Result<(), SendError> {
        let payload = serialize_payload(value)?;

        if let Some(recipient) = recipient {
            if !self.peers().any(|peer| peer.id == recipient) {
                return Err(SendError::UnknownPeer);
            }
        }

        let Ok(mut queue) = self.service.tx.try_lock() else {
            self.service.tx_queue_full.fetch_add(1, Ordering::Relaxed);
            return Err(SendError::QueueFull);
        };
        if !queue.push(recipient, payload.as_slice()) {
            self.service.tx_queue_full.fetch_add(1, Ordering::Relaxed);
            return Err(SendError::QueueFull);
        }
        Ok(())
    }
}

impl Runtime {
    pub(super) fn publish(self, mut snapshot: Snapshot) {
        snapshot.tx_queue_full = self.service.tx_queue_full.load(Ordering::Relaxed);
        snapshot.rx_queue_full = self.service.rx_queue_full.load(Ordering::Relaxed);
        self.service.latest.signal(snapshot);
    }

    pub(super) fn try_take_outgoing(self) -> Option<OutgoingMessage> {
        let mut queue = self.service.tx.try_lock().ok()?;
        queue.pop()
    }

    pub(super) fn push_incoming(
        self,
        sender: DeviceId,
        recipient: Option<DeviceId>,
        payload: &[u8],
    ) -> bool {
        let Ok(mut queue) = self.service.rx.try_lock() else {
            self.service.rx_queue_full.fetch_add(1, Ordering::Relaxed);
            return false;
        };
        if !queue.push(sender, recipient, payload) {
            self.service.rx_queue_full.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        true
    }
}
