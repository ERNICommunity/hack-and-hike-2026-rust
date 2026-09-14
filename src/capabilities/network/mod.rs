//! ESP-NOW peer discovery and typed messaging.
//!
//! Every board broadcasts a beacon a few times per second; boards that hear
//! each other become peers. Applications send and receive their own message
//! types through the [`Network`] handle, either to everyone
//! ([`Network::broadcast`]) or to one peer ([`Network::send_to`]).
//!
//! All boards use the same channel, so every board in the room hears every
//! message. Each message type carries a kind derived from its
//! [`Message::NAME`]; a board only decodes the kinds it knows.
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
//! - A message is at most [`MAX_PAYLOAD`] bytes once encoded.
//! - Delivery is not guaranteed; ESP-NOW is a radio in a noisy room. Send
//!   again, or send the current state instead of changes.
//! - Up to [`MAX_PEERS`] peers are tracked; beyond that the longest-silent
//!   one is replaced. Broadcasts reach every board regardless.
//!
//! The wire format, MAC addresses and the radio itself stay private; the
//! protocol and peer table live in `hack_and_hike_core::network`, where they
//! are tested on the host.

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

/// Messages the application may queue before the radio has sent them.
const OUTGOING_QUEUE_LENGTH: usize = 4;
/// Messages the radio may receive before the application reads them.
const INCOMING_QUEUE_LENGTH: usize = 4;

/// Radio configuration owned by the CPU1 network runtime.
#[derive(Clone, Copy)]
pub(crate) struct Config {
    /// The Wi-Fi channel every board uses. Boards on different channels do
    /// not hear each other.
    pub(crate) channel: RadioChannel,
    /// How often this board announces itself.
    pub(crate) beacon_period: Duration,
    /// A peer that stays silent this long is forgotten.
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

/// The radio, owned by CPU1.
pub(crate) struct Resources {
    /// The Wi-Fi peripheral, used for ESP-NOW only.
    pub(crate) wifi: WIFI<'static>,
}

/// The queues and counters shared by the handle (CPU0) and the radio tasks
/// (CPU1).
struct Service {
    /// The newest peer table and counters.
    latest: Signal<CriticalSectionRawMutex, Snapshot>,
    /// Messages waiting to be sent.
    outgoing: Channel<CriticalSectionRawMutex, OutgoingMessage, OUTGOING_QUEUE_LENGTH>,
    /// Messages waiting to be read.
    incoming: Channel<CriticalSectionRawMutex, IncomingMessage, INCOMING_QUEUE_LENGTH>,
    /// Messages refused because `outgoing` was full.
    tx_queue_full: AtomicU32,
    /// Messages dropped because `incoming` was full.
    rx_queue_full: AtomicU32,
}

static SERVICE: Service = Service {
    latest: Signal::new(),
    outgoing: Channel::new(),
    incoming: Channel::new(),
    tx_queue_full: AtomicU32::new(0),
    rx_queue_full: AtomicU32::new(0),
};

/// Application handle for ESP-NOW messaging; see the [module docs](self).
pub struct Network {
    service: &'static Service,
    /// The newest snapshot seen so far; refreshed whenever it is read.
    snapshot: Cell<Option<Snapshot>>,
}

impl Network {
    /// The newest peer table and counters, once the radio has published one.
    pub fn snapshot(&self) -> Option<Snapshot> {
        if let Some(snapshot) = self.service.latest.try_take() {
            self.snapshot.set(Some(snapshot));
        }
        self.snapshot.get()
    }

    /// The peers currently in range.
    pub fn peers(&self) -> impl Iterator<Item = Peer> {
        self.snapshot().into_iter().flat_map(Snapshot::into_peers)
    }

    /// The next received message, if any is waiting. Messages of kinds this
    /// application does not know still arrive here; check them with
    /// [`IncomingMessage::is`] or just let `decode` fail.
    pub fn next_message(&mut self) -> Option<IncomingMessage> {
        self.service.incoming.try_receive().ok()
    }

    /// Queue `value` for every board in range. Returns as soon as the
    /// message is queued; the radio sends it shortly after.
    ///
    /// # Errors
    ///
    /// [`SendError::MessageTooLarge`] when `value` encodes to more than
    /// [`MAX_PAYLOAD`] bytes, [`SendError::QueueFull`] when the application
    /// sends faster than the radio (try again on the next loop iteration).
    pub fn broadcast<T: Message>(&mut self, value: &T) -> Result<(), SendError> {
        self.enqueue(None, value)
    }

    /// Queue `value` for one peer, typically the sender of a message you
    /// received.
    ///
    /// # Errors
    ///
    /// [`SendError::UnknownPeer`] when that board is not in the current peer
    /// table, otherwise as for [`Network::broadcast`].
    pub fn send_to<T: Message>(&mut self, peer: DeviceId, value: &T) -> Result<(), SendError> {
        if !self.peers().any(|known| known.id == peer) {
            return Err(SendError::UnknownPeer);
        }
        self.enqueue(Some(peer), value)
    }

    /// Encode `value` and put it in the outgoing queue.
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

/// CPU1 side of the queues.
#[derive(Clone, Copy)]
pub(crate) struct Runtime {
    service: &'static Service,
}

impl Runtime {
    /// Make `snapshot` the newest one for the application.
    fn publish(self, snapshot: Snapshot) {
        self.service.latest.signal(snapshot);
    }

    /// The queue-full counters, which the handle updates on CPU0.
    fn queue_counters(self) -> QueueCounters {
        QueueCounters {
            tx_queue_full: self.service.tx_queue_full.load(Ordering::Relaxed),
            rx_queue_full: self.service.rx_queue_full.load(Ordering::Relaxed),
        }
    }

    /// Wait for the next message the application wants sent.
    async fn next_outgoing(self) -> OutgoingMessage {
        self.service.outgoing.receive().await
    }

    /// Hand a received message to the application, dropping it when the
    /// application has fallen behind.
    fn deliver(self, message: IncomingMessage) {
        if self.service.incoming.try_send(message).is_err() {
            self.service.rx_queue_full.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// The two ends of the network queues, created once by the board.
pub(crate) struct Endpoints {
    /// For the application.
    pub(crate) handle: Network,
    /// For the CPU1 radio tasks.
    pub(crate) runtime: Runtime,
}

/// Both ends of the network queues.
pub(crate) fn endpoints() -> Endpoints {
    Endpoints {
        handle: Network {
            service: &SERVICE,
            snapshot: Cell::new(None),
        },
        runtime: Runtime { service: &SERVICE },
    }
}
