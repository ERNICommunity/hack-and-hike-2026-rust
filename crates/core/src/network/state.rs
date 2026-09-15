//! Peer table and network status, independent of the radio.
//!
//! [`NetworkState`] lives on CPU1. Applications see it through a
//! [`Snapshot`].

use core::fmt;

use arrayvec::ArrayVec;

use super::protocol::{self, DeviceId, MacAddress};

/// The largest number of peers in the peer table. When the table is full, a
/// new peer replaces the peer that was silent the longest.
pub const MAX_PEERS: usize = 10;

/// A 2.4 GHz Wi-Fi channel number, 1 to 14. Boards hear each other only on
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

    /// A channel for a number written in the code. For a number computed at
    /// run time, use `RadioChannel::try_from`.
    ///
    /// # Panics
    ///
    /// When `number` is not between 1 and 14. In a `const`, the error
    /// appears at compile time.
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
    /// The radio is being set up.
    Starting,
    /// The radio works, but no peer is in range now.
    Ready,
    /// At least one peer is in range.
    PeerPresent,
    /// The radio failed to start. Nothing is sent or received.
    Fault,
}

/// One currently known peer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Peer {
    /// Which board it is. Pass it to `Network::send_to`.
    pub id: DeviceId,
    /// Signal strength of the last frame from this peer, in dBm. A value
    /// closer to zero means a stronger signal.
    pub rssi_dbm: i8,
    /// Milliseconds since the peer was last heard, when the snapshot was
    /// taken.
    pub age_ms: u32,
    /// Milliseconds until the peer is removed if it stays silent, when the
    /// snapshot was taken.
    pub expires_in_ms: u32,
    /// How long the peer has been running, from the uptime in its last
    /// beacon plus the time since. `None` until its first beacon arrives.
    pub uptime_ms: Option<u32>,
}

/// Peer table and counters as published by CPU1.
#[derive(Clone, Copy, Debug)]
pub struct Snapshot {
    /// Increases with every change to the peer table or the radio counters.
    /// Compare it with the last value to skip redrawing an unchanged
    /// snapshot. A change of only `tx_queue_full` or `rx_queue_full` does not
    /// change the revision.
    pub revision: u32,
    /// What the network is doing.
    pub status: Status,
    /// This board's own id.
    pub local_id: DeviceId,
    /// The channel all boards use.
    pub channel: RadioChannel,
    /// The peer table's slots; `None` is an empty slot. Read through
    /// [`Snapshot::peers`].
    peers: [Option<Peer>; MAX_PEERS],
    /// Frames sent successfully: beacons and messages.
    pub tx_packets: u32,
    /// Valid frames received from other boards.
    pub rx_packets: u32,
    /// Frames that were not sent: the radio failed, or the recipient was no
    /// longer in the peer table.
    pub tx_errors: u32,
    /// Received frames that are not valid frames of this protocol, or that
    /// are for another board.
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

    /// The peers in range, in no particular order. This consumes the
    /// snapshot.
    pub fn into_peers(self) -> impl Iterator<Item = Peer> {
        self.peers.into_iter().flatten()
    }
}

/// One row of the peer table, as CPU1 keeps it.
#[derive(Clone, Copy)]
struct PeerState {
    /// The peer's id, from its frames.
    device_id: DeviceId,
    /// The radio address for frames addressed only to this peer (unicast).
    mac: MacAddress,
    /// Signal strength of the last frame heard from the peer, in dBm.
    rssi_dbm: i8,
    /// When the peer was last heard, in milliseconds on this board's clock.
    last_seen_ms: u64,
    /// When the peer started, in milliseconds on this board's clock. It comes
    /// from the uptime in the peer's last beacon. Negative for a peer that
    /// started before this board. `None` until the first beacon.
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
    /// Signal strength of the frame, in dBm.
    pub rssi_dbm: i8,
    /// Uptime in milliseconds from a beacon; `None` for application messages.
    pub uptime_ms: Option<u32>,
}

/// What happened to the peer table when a frame arrived.
#[doc(hidden)]
pub struct Received {
    /// The sender was not a peer before this frame.
    pub is_new: bool,
    /// The radio address of a peer that was removed to make room for the
    /// sender.
    pub evicted: Option<MacAddress>,
}

/// The queue-full counters. They are kept outside [`NetworkState`] and
/// passed to [`NetworkState::snapshot`].
#[doc(hidden)]
#[derive(Clone, Copy, Default)]
pub struct QueueCounters {
    /// Messages refused because the send queue was full.
    pub tx_queue_full: u32,
    /// Messages dropped because the receive queue was full.
    pub rx_queue_full: u32,
}

/// The network as CPU1 sees it: the peer table and the counters.
#[doc(hidden)]
pub struct NetworkState {
    /// Increases with every change; becomes [`Snapshot::revision`].
    revision: u32,
    /// Whether the radio started.
    radio: RadioStatus,
    /// This board's own id.
    local_id: DeviceId,
    /// The channel all boards use.
    channel: RadioChannel,
    /// A peer that is silent for longer than this, in milliseconds, is
    /// removed.
    peer_timeout_ms: u64,
    /// Sequence number of the next beacon. Starts at 1.
    next_sequence: u32,
    /// The peer table's slots; `None` is an empty slot.
    peers: [Option<PeerState>; MAX_PEERS],
    /// See [`Snapshot::tx_packets`].
    tx_packets: u32,
    /// See [`Snapshot::rx_packets`].
    rx_packets: u32,
    /// See [`Snapshot::tx_errors`].
    tx_errors: u32,
    /// See [`Snapshot::rx_invalid`].
    rx_invalid: u32,
    /// See [`Snapshot::peer_evictions`].
    peer_evictions: u32,
}

/// The state of the radio itself. Whether peers are present comes from the
/// peer table when a snapshot is taken.
#[derive(Clone, Copy)]
enum RadioStatus {
    /// Neither `mark_ready` nor `mark_fault` was called yet.
    Starting,
    /// The radio started.
    Ready,
    /// The radio failed to start.
    Fault,
}

impl NetworkState {
    /// An empty peer table for this board. Peers that are silent for longer
    /// than `peer_timeout_ms` are removed by
    /// [`expire_peers`](Self::expire_peers).
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

    /// Record that the radio started.
    pub fn mark_ready(&mut self) {
        self.radio = RadioStatus::Ready;
        self.bump_revision();
    }

    /// Record that the radio failed to start.
    pub fn mark_fault(&mut self) {
        self.radio = RadioStatus::Fault;
        self.bump_revision();
    }

    /// Note a change for the next snapshot.
    fn bump_revision(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    /// The next beacon to send, with the next sequence number and the uptime
    /// `now_ms`.
    pub fn next_beacon(&mut self, now_ms: u64) -> protocol::BeaconPacket {
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.wrapping_add(1);
        protocol::BeaconPacket {
            device_id: self.local_id,
            sequence,
            // The uptime wraps around after about 49 days. That is good
            // enough for an uptime display.
            uptime_ms: (now_ms % (u64::from(u32::MAX) + 1)) as u32,
        }
    }

    /// Count a frame that was sent.
    pub fn record_send_ok(&mut self) {
        self.tx_packets = self.tx_packets.wrapping_add(1);
        self.bump_revision();
    }

    /// Count a frame that was not sent: the radio failed, or the recipient
    /// has no route.
    pub fn record_send_error(&mut self) {
        self.tx_errors = self.tx_errors.wrapping_add(1);
        self.bump_revision();
    }

    /// Count a received frame that is not a valid frame of this protocol, or
    /// an application frame for another board.
    pub fn record_invalid_receive(&mut self) {
        self.rx_invalid = self.rx_invalid.wrapping_add(1);
        self.bump_revision();
    }

    /// The radio address for a frame to `device_id` only. `None` when that
    /// board is not in the peer table.
    pub fn route_for(&self, device_id: DeviceId) -> Option<MacAddress> {
        self.peers
            .iter()
            .flatten()
            .find(|peer| peer.device_id == device_id)
            .map(|peer| peer.mac)
    }

    /// Add the sender of a valid frame to the peer table, or update its row.
    /// Also counts the frame in `rx_packets`.
    ///
    /// When the table is full, a new peer replaces the peer that was silent
    /// the longest.
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
            // An application message has no uptime. Keep the start time from
            // the peer's last beacon.
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

    /// The index of a free slot. When there is none, the peer that was
    /// silent the longest is removed and counted in `peer_evictions`. Then
    /// its slot and its radio address are returned.
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

    /// Remove the peers that were silent for longer than the timeout, and
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

    /// The current state for applications, at the time `now_ms`.
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

/// When a peer started, in milliseconds on this board's clock: now minus
/// its reported uptime. Negative when it started before this board.
fn started_at(now_ms: u64, uptime_ms: u32) -> i64 {
    now_ms as i64 - i64::from(uptime_ms)
}

/// Milliseconds as `u32`: negative becomes 0, too large becomes
/// `u32::MAX`.
fn saturate_signed(ms: i64) -> u32 {
    u32::try_from(ms.max(0)).unwrap_or(u32::MAX)
}

/// Milliseconds as `u32`, capped at `u32::MAX`.
fn saturate(ms: u64) -> u32 {
    u32::try_from(ms).unwrap_or(u32::MAX)
}
