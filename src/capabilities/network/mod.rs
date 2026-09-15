//! Finding nearby boards and sending typed messages over ESP-NOW.
//!
//! ESP-NOW is a direct radio protocol from Espressif, the maker of the chip.
//! It needs no Wi-Fi access point. Every board sends a beacon four times per
//! second. When this board hears another board, that board becomes a
//! *peer*. A peer that stays silent for more than half a second is removed.
//!
//! Applications send and receive their own message types through the
//! [`Network`] handle. A message goes either to all boards
//! ([`Network::broadcast`]) or to one peer ([`Network::send_to`]).
//!
//! All boards use the same radio channel, so every board in the room
//! receives every broadcast. Each message type has a *kind*: a number
//! calculated from its [`Message::NAME`].
//! [`IncomingMessage::decode`] only accepts a message whose kind matches the
//! type.
//!
//! ```ignore
//! #[derive(Serialize, Deserialize)]
//! struct Hello {
//!     number: u32,
//! }
//!
//! impl Message for Hello {
//!     const NAME: &'static str = "team-otters.hello";
//! }
//!
//! if let Err(error) = network.broadcast(&Hello { number: 42 }) {
//!     log::warn!("not sent: {error}");
//! }
//! while let Some(message) = network.next_message() {
//!     if let Ok(hello) = message.decode::<Hello>() { /* ... */ }
//! }
//! ```
//!
//! # Limits
//!
//! - A message is at most [`MAX_PAYLOAD`] bytes after encoding.
//! - Delivery is not guaranteed: radio messages can get lost, especially in
//!   a busy room. Send important messages again, or send the complete
//!   current state instead of changes.
//! - Up to four messages wait to be sent, and up to four received messages
//!   wait to be read. When the receive queue is full, new messages are
//!   dropped, and [`Snapshot::rx_queue_full`] counts them. So read all
//!   waiting messages on every loop iteration.
//! - The peer table has at most [`MAX_PEERS`] peers. When it is full, a new
//!   peer replaces the peer that was silent the longest. Broadcasts still
//!   reach every board.
//!
//! The wire format, the MAC addresses (the hardware addresses of the radios)
//! and the radio itself stay private. The
//! protocol and the peer table are in `hack_and_hike_core::network`, where
//! they are tested on the host computer.

mod runtime;

use core::{
    cell::Cell,
    sync::atomic::{AtomicU32, Ordering},
};

use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex, channel::Channel, signal::Signal,
};
use embassy_time::Duration;
use esp_hal::peripherals::WIFI;

use hack_and_hike_core::network::{message::OutgoingMessage, state::QueueCounters};

pub use hack_and_hike_core::network::{
    message::{DecodeError, IncomingMessage, Message, SendError},
    protocol::{DeviceId, MAX_PAYLOAD},
    state::{MAX_PEERS, Peer, RadioChannel, Snapshot, Status},
};

pub(crate) use runtime::spawn;

/// Messages that can wait in the send queue before the radio sends them.
const OUTGOING_QUEUE_LENGTH: usize = 4;
/// Received messages that can wait in the receive queue before the
/// application reads them.
const INCOMING_QUEUE_LENGTH: usize = 4;

/// Radio settings for the CPU1 network tasks.
#[derive(Clone, Copy)]
pub(crate) struct Config {
    /// The Wi-Fi channel that every board uses. Boards on different channels
    /// do not hear each other.
    pub(crate) channel: RadioChannel,
    /// Time between two beacons of this board.
    pub(crate) beacon_period: Duration,
    /// A peer that stays silent for longer than this is removed. The check
    /// runs once per beacon period, so the removal can happen up to one
    /// beacon period later.
    pub(crate) peer_timeout: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            channel: RadioChannel::new(6),
            beacon_period: Duration::from_millis(250),
            peer_timeout: Duration::from_millis(500),
        }
    }
}

/// The radio hardware that the CPU1 network tasks use.
pub(crate) struct Resources {
    /// The Wi-Fi peripheral. Only ESP-NOW uses it.
    pub(crate) wifi: WIFI<'static>,
}

/// The queues and counters that the handle (CPU0) and the radio tasks (CPU1)
/// share.
struct Service {
    /// The newest snapshot: peer table and counters.
    latest: Signal<CriticalSectionRawMutex, Snapshot>,
    /// The send queue: messages that wait to be sent.
    outgoing: Channel<CriticalSectionRawMutex, OutgoingMessage, OUTGOING_QUEUE_LENGTH>,
    /// The receive queue: messages that wait to be read.
    incoming: Channel<CriticalSectionRawMutex, IncomingMessage, INCOMING_QUEUE_LENGTH>,
    /// Messages refused because `outgoing` was full. The handle on CPU0
    /// counts them.
    tx_queue_full: AtomicU32,
    /// Messages dropped because `incoming` was full. The receive task on
    /// CPU1 counts them.
    rx_queue_full: AtomicU32,
}

/// The only set of network queues and counters.
///
/// A plain `static` works on both CPUs, because every field is safe to use
/// from several CPUs: the `Signal` and the `Channel`s use a critical section,
/// and the counters are atomic.
static SERVICE: Service = Service {
    latest: Signal::new(),
    outgoing: Channel::new(),
    incoming: Channel::new(),
    tx_queue_full: AtomicU32::new(0),
    rx_queue_full: AtomicU32::new(0),
};

/// Application handle for ESP-NOW messaging; see the [module docs](self).
pub struct Network {
    /// The queues and counters shared with the CPU1 radio tasks.
    service: &'static Service,
    /// The newest snapshot so far. [`Network::snapshot`] updates it when a
    /// newer snapshot is available. It is a `Cell`, so a method with `&self`
    /// can update it.
    snapshot: Cell<Option<Snapshot>>,
}

impl Network {
    /// The newest peer table and counters. `None` until the radio task has
    /// published the first snapshot. Never waits.
    pub fn snapshot(&self) -> Option<Snapshot> {
        if let Some(snapshot) = self.service.latest.try_take() {
            self.snapshot.set(Some(snapshot));
        }
        self.snapshot.get()
    }

    /// The peers in range, from the newest snapshot.
    pub fn peers(&self) -> impl Iterator<Item = Peer> {
        self.snapshot().into_iter().flat_map(Snapshot::into_peers)
    }

    /// The oldest received message that is waiting, or `None`. Never waits.
    ///
    /// Messages of kinds that this application does not know also arrive
    /// here. Check the kind with [`IncomingMessage::is`], or let
    /// [`IncomingMessage::decode`] return an error.
    pub fn next_message(&mut self) -> Option<IncomingMessage> {
        self.service.incoming.try_receive().ok()
    }

    /// Queue `value` to be sent to all boards in range. Return as soon as
    /// the message is in the queue. The radio sends it shortly after.
    ///
    /// # Errors
    ///
    /// - [`SendError::MessageTooLarge`] when `value` encodes to more than
    ///   [`MAX_PAYLOAD`] bytes.
    /// - [`SendError::QueueFull`] when the application sends faster than the
    ///   radio. Try again on the next loop iteration.
    pub fn broadcast<T: Message>(&mut self, value: &T) -> Result<(), SendError> {
        self.enqueue(None, value)
    }

    /// Queue `value` to be sent to one peer, for example to the sender of a
    /// message you received.
    ///
    /// When the peer leaves the peer table before the radio sends the
    /// message, the message is not sent, and [`Snapshot::tx_errors`] counts
    /// it.
    ///
    /// # Errors
    ///
    /// [`SendError::UnknownPeer`] when `peer` is not in the newest peer
    /// table. The other errors are the same as for [`Network::broadcast`].
    pub fn send_to<T: Message>(&mut self, peer: DeviceId, value: &T) -> Result<(), SendError> {
        if !self.peers().any(|known| known.id == peer) {
            return Err(SendError::UnknownPeer);
        }
        self.enqueue(Some(peer), value)
    }

    /// Encode `value` and put it in the send queue. `recipient` is `None` for
    /// a broadcast.
    ///
    /// # Errors
    ///
    /// [`SendError::MessageTooLarge`] or [`SendError::QueueFull`]. A full
    /// queue also increases the `tx_queue_full` counter.
    fn enqueue<T: Message>(
        &mut self,
        recipient: Option<DeviceId>,
        value: &T,
    ) -> Result<(), SendError> {
        let message = OutgoingMessage::new(recipient, value)?;
        self.service.outgoing.try_send(message).map_err(|_| {
            self.service.tx_queue_full.fetch_add(1, Ordering::Relaxed);
            SendError::QueueFull
        })
    }
}

/// The CPU1 side of the network queues. The radio tasks use it.
#[derive(Clone, Copy)]
pub(crate) struct Runtime {
    /// The queues and counters shared with the application's handle on CPU0.
    service: &'static Service,
}

impl Runtime {
    /// Make `snapshot` the newest snapshot for the application.
    fn publish(self, snapshot: Snapshot) {
        self.service.latest.signal(snapshot);
    }

    /// The current queue-full counters. The handle on CPU0 counts
    /// `tx_queue_full`, and [`Runtime::deliver`] on CPU1 counts
    /// `rx_queue_full`.
    fn queue_counters(self) -> QueueCounters {
        QueueCounters {
            tx_queue_full: self.service.tx_queue_full.load(Ordering::Relaxed),
            rx_queue_full: self.service.rx_queue_full.load(Ordering::Relaxed),
        }
    }

    /// Wait for the next message in the send queue.
    async fn next_outgoing(self) -> OutgoingMessage {
        self.service.outgoing.receive().await
    }

    /// Put a received message in the receive queue for the application.
    ///
    /// When the queue is full, the message is dropped and `rx_queue_full`
    /// increases. This does not wait.
    fn deliver(self, message: IncomingMessage) {
        if self.service.incoming.try_send(message).is_err() {
            self.service.rx_queue_full.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// The two ends of the network queues. The board creates them once.
pub(crate) struct Endpoints {
    /// For the application.
    pub(crate) handle: Network,
    /// For the CPU1 radio tasks.
    pub(crate) runtime: Runtime,
}

/// Create both ends of the network queues.
pub(crate) fn endpoints() -> Endpoints {
    Endpoints {
        handle: Network {
            service: &SERVICE,
            snapshot: Cell::new(None),
        },
        runtime: Runtime { service: &SERVICE },
    }
}
