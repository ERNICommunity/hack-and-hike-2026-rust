//! Peer table and network status, independent of the radio.
//!
//! [`NetworkState`] lives on CPU1; applications see it through [`Snapshot`].

use core::fmt;

use arrayvec::ArrayVec;

use super::protocol::{self, DeviceId, MacAddress, RssiDbm};

/// Most peers tracked at once. The longest-unseen peer is replaced when full.
pub const MAX_PEERS: usize = 10;

/// A 2.4 GHz ESP-NOW channel number (1 to 14).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Channel(u8);

impl Channel {
    /// Panics on a channel outside 1 to 14.
    pub const fn new(number: u8) -> Self {
        assert!(
            number >= 1 && number <= 14,
            "ESP-NOW channel must be 1 to 14"
        );
        Self(number)
    }

    pub const fn number(self) -> u8 {
        self.0
    }
}

impl fmt::Display for Channel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

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
    pub id: DeviceId,
    pub rssi_dbm: i8,
    /// Time since the peer was last heard.
    pub age_ms: u32,
    /// Time until the peer is forgotten if it stays silent.
    pub expires_in_ms: u32,
}

/// Peer table and counters as published by CPU1.
#[derive(Clone, Copy, Debug)]
pub struct Snapshot {
    /// Increments with every change to the peer table or counters.
    pub revision: u32,
    pub status: Status,
    pub local_id: DeviceId,
    pub channel: Channel,
    peers: [Option<Peer>; MAX_PEERS],
    pub tx_packets: u32,
    pub rx_packets: u32,
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
    pub fn peer_count(&self) -> usize {
        self.peers().count()
    }

    pub fn peers(&self) -> impl Iterator<Item = &Peer> {
        self.peers.iter().flatten()
    }
}

#[derive(Clone, Copy)]
struct PeerState {
    device_id: DeviceId,
    mac: MacAddress,
    rssi_dbm: RssiDbm,
    last_seen_ms: u64,
}

/// What happened to the peer table when a frame arrived.
pub struct Received {
    /// The sender was not a peer before this frame.
    pub is_new: bool,
    /// A peer that was replaced to make room for the sender.
    pub evicted: Option<MacAddress>,
}

/// Queue-full counters kept outside the state, passed in for the snapshot.
#[derive(Clone, Copy, Default)]
pub struct QueueCounters {
    pub tx_queue_full: u32,
    pub rx_queue_full: u32,
}

pub struct NetworkState {
    revision: u32,
    status: Status,
    local_id: DeviceId,
    channel: Channel,
    peer_timeout_ms: u64,
    next_sequence: u32,
    peers: [Option<PeerState>; MAX_PEERS],
    tx_packets: u32,
    rx_packets: u32,
    tx_errors: u32,
    rx_invalid: u32,
    peer_evictions: u32,
}

impl NetworkState {
    pub fn new(local_id: DeviceId, channel: Channel, peer_timeout_ms: u64) -> Self {
        Self {
            revision: 0,
            status: Status::Starting,
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

    pub fn mark_ready(&mut self) {
        self.status = Status::Ready;
        self.bump_revision();
    }

    pub fn mark_fault(&mut self) {
        self.status = Status::Fault;
        self.bump_revision();
    }

    fn bump_revision(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    pub fn next_beacon(&mut self, now_ms: u64) -> protocol::BeaconPacket {
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.wrapping_add(1);
        protocol::BeaconPacket {
            device_id: self.local_id,
            sequence,
            // Wraps after 49 days, which is fine for an uptime hint.
            uptime_ms: u32::try_from(now_ms % (u64::from(u32::MAX) + 1)).unwrap_or(u32::MAX),
        }
    }

    pub fn record_send_ok(&mut self) {
        self.tx_packets = self.tx_packets.wrapping_add(1);
        self.bump_revision();
    }

    pub fn record_send_error(&mut self) {
        self.tx_errors = self.tx_errors.wrapping_add(1);
        self.bump_revision();
    }

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

    pub fn record_receive(
        &mut self,
        device_id: DeviceId,
        mac: MacAddress,
        rssi_dbm: RssiDbm,
        now_ms: u64,
    ) -> Received {
        self.rx_packets = self.rx_packets.wrapping_add(1);
        self.bump_revision();

        if let Some(peer) = self
            .peers
            .iter_mut()
            .flatten()
            .find(|peer| peer.device_id == device_id)
        {
            peer.mac = mac;
            peer.rssi_dbm = rssi_dbm;
            peer.last_seen_ms = now_ms;
            return Received {
                is_new: false,
                evicted: None,
            };
        }

        let (index, evicted) = self.slot_for_new_peer();
        self.peers[index] = Some(PeerState {
            device_id,
            mac,
            rssi_dbm,
            last_seen_ms: now_ms,
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
    pub fn snapshot(&mut self, now_ms: u64, queues: QueueCounters) -> Snapshot {
        let mut peers = [None; MAX_PEERS];
        for (target, source) in peers.iter_mut().zip(self.peers.iter().flatten()) {
            let age_ms = now_ms.saturating_sub(source.last_seen_ms);
            *target = Some(Peer {
                id: source.device_id,
                rssi_dbm: source.rssi_dbm.0,
                age_ms: saturate(age_ms),
                expires_in_ms: saturate(self.peer_timeout_ms.saturating_sub(age_ms)),
            });
        }

        if matches!(self.status, Status::Ready | Status::PeerPresent) {
            self.status = if peers.iter().flatten().next().is_none() {
                Status::Ready
            } else {
                Status::PeerPresent
            };
        }

        Snapshot {
            revision: self.revision,
            status: self.status,
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

fn saturate(ms: u64) -> u32 {
    u32::try_from(ms).unwrap_or(u32::MAX)
}
