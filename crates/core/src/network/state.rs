//! Peer table and network status, independent of the radio.
//!
//! [`NetworkState`] lives on CPU1; applications see it through [`Snapshot`].

use core::fmt;

use arrayvec::ArrayVec;

use super::protocol::{self, DeviceId, MacAddress};

/// Most peers tracked at once. The longest-unseen peer is replaced when full.
pub const MAX_PEERS: usize = 10;

/// A 2.4 GHz Wi-Fi channel number, 1 to 14. Boards only hear each other on
/// the same channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RadioChannel(u8);

/// The channel number was outside 1 to 14.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvalidChannel;

impl RadioChannel {
    /// The lowest channel number.
    pub const MIN: u8 = 1;
    /// The highest channel number.
    pub const MAX: u8 = 14;

    /// For channel numbers written in the code. Panics outside 1 to 14, so
    /// a typo fails at compile time in a `const`.
    pub const fn new(number: u8) -> Self {
        assert!(
            number >= Self::MIN && number <= Self::MAX,
            "Wi-Fi channel must be 1 to 14"
        );
        Self(number)
    }

    /// The channel number, 1 to 14.
    pub const fn number(self) -> u8 {
        self.0
    }
}

/// For channel numbers computed at run time.
impl TryFrom<u8> for RadioChannel {
    type Error = InvalidChannel;

    fn try_from(number: u8) -> Result<Self, Self::Error> {
        if (Self::MIN..=Self::MAX).contains(&number) {
            Ok(Self(number))
        } else {
            Err(InvalidChannel)
        }
    }
}

impl fmt::Display for RadioChannel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// What the network is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// The radio is being initialized.
    Starting,
    /// The radio works but no peer has been heard.
    Ready,
    /// At least one peer is in range.
    PeerPresent,
    /// The radio failed to initialize; nothing will be sent or received.
    Fault,
}

/// One currently known peer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Peer {
    /// Which board it is; pass it to `Network::send_to`.
    pub id: DeviceId,
    /// Received signal strength in dBm; closer to zero is stronger.
    pub rssi_dbm: i8,
    /// Time since the peer was last heard.
    pub age_ms: u32,
    /// Time until the peer is forgotten if it stays silent.
    pub expires_in_ms: u32,
    /// How long the peer has been running, from the uptime in its last
    /// beacon plus the time since. `None` until its first beacon arrives.
    pub uptime_ms: Option<u32>,
}

/// Peer table and counters as published by CPU1.
#[derive(Clone, Copy, Debug)]
pub struct Snapshot {
    /// Increments with every change to the peer table or counters; compare
    /// it to skip redrawing an unchanged snapshot.
    pub revision: u32,
    /// What the network is doing.
    pub status: Status,
    /// This board's own id.
    pub local_id: DeviceId,
    /// The channel all boards use.
    pub channel: RadioChannel,
    peers: [Option<Peer>; MAX_PEERS],
    /// Frames sent successfully: beacons and messages.
    pub tx_packets: u32,
    /// Frames received from other boards.
    pub rx_packets: u32,
    /// Frames the radio failed to send.
    pub tx_errors: u32,
    /// Frames that were not valid frames of this protocol.
    pub rx_invalid: u32,
    /// Peers replaced because the table was full.
    pub peer_evictions: u32,
    /// Messages refused because the send queue was full.
    pub tx_queue_full: u32,
    /// Messages dropped because the receive queue was full.
    pub rx_queue_full: u32,
}

impl Snapshot {
    /// How many peers are in range.
    pub fn peer_count(&self) -> usize {
        self.peers().count()
    }

    /// The peers in range, in no particular order.
    pub fn peers(&self) -> impl Iterator<Item = &Peer> {
        self.peers.iter().flatten()
    }

    /// The peers in range, taking the snapshot.
    pub fn into_peers(self) -> impl Iterator<Item = Peer> {
        self.peers.into_iter().flatten()
    }
}

/// One row of the peer table, as the radio keeps it.
#[derive(Clone, Copy)]
struct PeerState {
    device_id: DeviceId,
    /// Where to send unicast frames for this peer.
    mac: MacAddress,
    rssi_dbm: i8,
    /// When the peer was last heard, on our clock.
    last_seen_ms: u64,
    /// When the peer booted on our clock, worked out from the uptime in its
    /// last beacon. Negative for a peer that booted before we did.
    started_at_ms: Option<i64>,
}

/// What one received frame says about the device that sent it.
#[doc(hidden)]
#[derive(Clone, Copy, Debug)]
pub struct Heard {
    /// The sender's id from the frame.
    pub device_id: DeviceId,
    /// The sender's radio address.
    pub mac: MacAddress,
    /// Signal strength of the frame.
    pub rssi_dbm: i8,
    /// Uptime carried by a beacon; `None` for application messages.
    pub uptime_ms: Option<u32>,
}

/// What happened to the peer table when a frame arrived.
#[doc(hidden)]
pub struct Received {
    /// The sender was not a peer before this frame.
    pub is_new: bool,
    /// A peer that was replaced to make room for the sender.
    pub evicted: Option<MacAddress>,
}

/// Queue-full counters kept outside the state, passed in for the snapshot.
#[doc(hidden)]
#[derive(Clone, Copy, Default)]
pub struct QueueCounters {
    /// Messages refused because the send queue was full.
    pub tx_queue_full: u32,
    /// Messages dropped because the receive queue was full.
    pub rx_queue_full: u32,
}

/// The radio's view of the network: the peer table and the counters.
#[doc(hidden)]
pub struct NetworkState {
    revision: u32,
    radio: RadioStatus,
    local_id: DeviceId,
    channel: RadioChannel,
    peer_timeout_ms: u64,
    next_sequence: u32,
    peers: [Option<PeerState>; MAX_PEERS],
    tx_packets: u32,
    rx_packets: u32,
    tx_errors: u32,
    rx_invalid: u32,
    peer_evictions: u32,
}

/// The radio's own state; whether peers are present is derived from the
/// peer table when a snapshot is taken.
#[derive(Clone, Copy)]
enum RadioStatus {
    Starting,
    Ready,
    Fault,
}

impl NetworkState {
    /// An empty peer table for this board; peers silent for longer than
    /// `peer_timeout_ms` are forgotten by [`expire_peers`](Self::expire_peers).
    pub fn new(local_id: DeviceId, channel: RadioChannel, peer_timeout_ms: u64) -> Self {
        Self {
            revision: 0,
            radio: RadioStatus::Starting,
            local_id,
            channel,
            peer_timeout_ms,
            next_sequence: 1,
            peers: [None; MAX_PEERS],
            tx_packets: 0,
            rx_packets: 0,
            tx_errors: 0,
            rx_invalid: 0,
            peer_evictions: 0,
        }
    }

    /// The radio came up.
    pub fn mark_ready(&mut self) {
        self.radio = RadioStatus::Ready;
        self.bump_revision();
    }

    /// The radio failed to come up.
    pub fn mark_fault(&mut self) {
        self.radio = RadioStatus::Fault;
        self.bump_revision();
    }

    /// Note a change for the next snapshot.
    fn bump_revision(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    /// The next beacon to send, numbered.
    pub fn next_beacon(&mut self, now_ms: u64) -> protocol::BeaconPacket {
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.wrapping_add(1);
        protocol::BeaconPacket {
            device_id: self.local_id,
            sequence,
            // Wraps after 49 days, which is fine for an uptime hint.
            uptime_ms: (now_ms % (u64::from(u32::MAX) + 1)) as u32,
        }
    }

    /// Count a frame the radio sent.
    pub fn record_send_ok(&mut self) {
        self.tx_packets = self.tx_packets.wrapping_add(1);
        self.bump_revision();
    }

    /// Count a frame the radio failed to send.
    pub fn record_send_error(&mut self) {
        self.tx_errors = self.tx_errors.wrapping_add(1);
        self.bump_revision();
    }

    /// Count a received frame that is not ours or is malformed.
    pub fn record_invalid_receive(&mut self) {
        self.rx_invalid = self.rx_invalid.wrapping_add(1);
        self.bump_revision();
    }

    /// The radio address to use for a unicast to `device_id`.
    pub fn route_for(&self, device_id: DeviceId) -> Option<MacAddress> {
        self.peers
            .iter()
            .flatten()
            .find(|peer| peer.device_id == device_id)
            .map(|peer| peer.mac)
    }

    /// Add or refresh the sender of a valid frame. A new peer may replace the
    /// longest-silent one when the table is full.
    pub fn record_receive(&mut self, heard: Heard, now_ms: u64) -> Received {
        self.rx_packets = self.rx_packets.wrapping_add(1);
        self.bump_revision();

        if let Some(peer) = self
            .peers
            .iter_mut()
            .flatten()
            .find(|peer| peer.device_id == heard.device_id)
        {
            peer.mac = heard.mac;
            peer.rssi_dbm = heard.rssi_dbm;
            peer.last_seen_ms = now_ms;
            // An application message says nothing about uptime; keep what
            // the peer's last beacon said.
            if let Some(uptime_ms) = heard.uptime_ms {
                peer.started_at_ms = Some(started_at(now_ms, uptime_ms));
            }
            return Received {
                is_new: false,
                evicted: None,
            };
        }

        let (index, evicted) = self.slot_for_new_peer();
        self.peers[index] = Some(PeerState {
            device_id: heard.device_id,
            mac: heard.mac,
            rssi_dbm: heard.rssi_dbm,
            last_seen_ms: now_ms,
            started_at_ms: heard
                .uptime_ms
                .map(|uptime_ms| started_at(now_ms, uptime_ms)),
        });
        Received {
            is_new: true,
            evicted,
        }
    }

    /// A free slot, or the slot of the longest-unseen peer, which is evicted.
    fn slot_for_new_peer(&mut self) -> (usize, Option<MacAddress>) {
        let mut oldest_index = 0;
        let mut oldest_seen = u64::MAX;

        for (index, peer) in self.peers.iter().enumerate() {
            let Some(peer) = peer else {
                return (index, None);
            };
            if peer.last_seen_ms < oldest_seen {
                oldest_seen = peer.last_seen_ms;
                oldest_index = index;
            }
        }

        self.peer_evictions = self.peer_evictions.wrapping_add(1);
        let evicted = self.peers[oldest_index].take().map(|peer| peer.mac);
        (oldest_index, evicted)
    }

    /// Forget peers that have been silent for longer than the timeout and
    /// return their radio addresses.
    pub fn expire_peers(&mut self, now_ms: u64) -> ArrayVec<MacAddress, MAX_PEERS> {
        let mut expired = ArrayVec::new();
        for slot in &mut self.peers {
            if slot
                .as_ref()
                .is_some_and(|peer| now_ms.saturating_sub(peer.last_seen_ms) > self.peer_timeout_ms)
                && let Some(peer) = slot.take()
            {
                expired.push(peer.mac);
            }
        }
        if !expired.is_empty() {
            self.bump_revision();
        }
        expired
    }

    /// The current state as seen by applications.
    pub fn snapshot(&self, now_ms: u64, queues: QueueCounters) -> Snapshot {
        let mut peers = [None; MAX_PEERS];
        for (target, source) in peers.iter_mut().zip(self.peers.iter().flatten()) {
            let age_ms = now_ms.saturating_sub(source.last_seen_ms);
            *target = Some(Peer {
                id: source.device_id,
                rssi_dbm: source.rssi_dbm,
                age_ms: saturate(age_ms),
                expires_in_ms: saturate(self.peer_timeout_ms.saturating_sub(age_ms)),
                uptime_ms: source
                    .started_at_ms
                    .map(|started_at_ms| saturate_signed(now_ms as i64 - started_at_ms)),
            });
        }

        let status = match self.radio {
            RadioStatus::Starting => Status::Starting,
            RadioStatus::Fault => Status::Fault,
            RadioStatus::Ready if peers.iter().flatten().next().is_none() => Status::Ready,
            RadioStatus::Ready => Status::PeerPresent,
        };

        Snapshot {
            revision: self.revision,
            status,
            local_id: self.local_id,
            channel: self.channel,
            peers,
            tx_packets: self.tx_packets,
            rx_packets: self.rx_packets,
            tx_errors: self.tx_errors,
            rx_invalid: self.rx_invalid,
            peer_evictions: self.peer_evictions,
            tx_queue_full: queues.tx_queue_full,
            rx_queue_full: queues.rx_queue_full,
        }
    }
}

fn started_at(now_ms: u64, uptime_ms: u32) -> i64 {
    now_ms as i64 - i64::from(uptime_ms)
}

fn saturate_signed(ms: i64) -> u32 {
    u32::try_from(ms.max(0)).unwrap_or(u32::MAX)
}

fn saturate(ms: u64) -> u32 {
    u32::try_from(ms).unwrap_or(u32::MAX)
}
