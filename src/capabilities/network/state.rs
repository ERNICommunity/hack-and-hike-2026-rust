//! Runtime-independent peer tracking, routing and network state transitions.

use super::{Channel, MAX_PEERS, MacAddress, Peer, RssiDbm, Snapshot, Status, protocol};

#[derive(Clone, Copy)]
struct PeerState {
    device_id: protocol::DeviceId,
    mac: MacAddress,
    rssi_dbm: RssiDbm,
    last_seen_ms: u64,
    rx_packets: u32,
}

pub(super) struct NetworkState {
    revision: u32,
    status: Status,
    local_id: protocol::DeviceId,
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
    pub(super) fn new(
        local_id: protocol::DeviceId,
        channel: Channel,
        peer_timeout_ms: u64,
    ) -> Self {
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

    pub(super) fn mark_ready(&mut self) {
        self.status = Status::Ready;
        self.bump_revision();
    }

    pub(super) fn mark_fault(&mut self) {
        self.status = Status::Fault;
        self.bump_revision();
    }

    fn bump_revision(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    pub(super) fn next_beacon(
        &mut self,
        now_ms: u64,
        capabilities: protocol::Capabilities,
    ) -> protocol::BeaconPacket {
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.wrapping_add(1);
        protocol::BeaconPacket::new(self.local_id, sequence, now_ms as u32, capabilities)
    }

    pub(super) fn record_send_ok(&mut self) {
        self.tx_packets = self.tx_packets.wrapping_add(1);
        self.bump_revision();
    }

    pub(super) fn record_send_error(&mut self) {
        self.tx_errors = self.tx_errors.wrapping_add(1);
        self.bump_revision();
    }

    pub(super) fn record_invalid_receive(&mut self) {
        self.rx_invalid = self.rx_invalid.wrapping_add(1);
        self.bump_revision();
    }

    pub(super) fn route_for(&self, device_id: protocol::DeviceId) -> Option<MacAddress> {
        self.peers
            .iter()
            .flatten()
            .find(|peer| peer.device_id == device_id)
            .map(|peer| peer.mac)
    }

    pub(super) fn record_receive(
        &mut self,
        device_id: protocol::DeviceId,
        mac: MacAddress,
        rssi_dbm: RssiDbm,
        now_ms: u64,
    ) -> bool {
        self.rx_packets = self.rx_packets.wrapping_add(1);

        if let Some(peer) = self
            .peers
            .iter_mut()
            .flatten()
            .find(|peer| peer.device_id == device_id)
        {
            peer.mac = mac;
            peer.rssi_dbm = rssi_dbm;
            peer.last_seen_ms = now_ms;
            peer.rx_packets = peer.rx_packets.wrapping_add(1);
            self.bump_revision();
            return false;
        }

        let index = self.slot_for_new_peer();
        self.peers[index] = Some(PeerState {
            device_id,
            mac,
            rssi_dbm,
            last_seen_ms: now_ms,
            rx_packets: 1,
        });
        self.bump_revision();
        true
    }

    /// Index of a free peer slot, evicting the longest-unseen peer if needed.
    fn slot_for_new_peer(&mut self) -> usize {
        let mut oldest_index = 0usize;
        let mut oldest_seen = u64::MAX;

        for (index, peer) in self.peers.iter().enumerate() {
            let Some(peer) = peer else {
                return index;
            };
            if peer.last_seen_ms < oldest_seen {
                oldest_seen = peer.last_seen_ms;
                oldest_index = index;
            }
        }

        self.peer_evictions = self.peer_evictions.wrapping_add(1);
        oldest_index
    }

    fn expire_peers(&mut self, now_ms: u64) {
        let mut changed = false;
        for peer in &mut self.peers {
            if peer
                .as_ref()
                .is_some_and(|peer| now_ms.saturating_sub(peer.last_seen_ms) > self.peer_timeout_ms)
            {
                *peer = None;
                changed = true;
            }
        }
        if changed {
            self.bump_revision();
        }
    }

    pub(super) fn snapshot(&mut self, now_ms: u64) -> Snapshot {
        self.expire_peers(now_ms);
        let peer_count = self.peers.iter().flatten().count();
        let mut peers = [None; MAX_PEERS];

        for (target, source) in peers.iter_mut().zip(self.peers.iter().flatten()) {
            let age_ms = now_ms.saturating_sub(source.last_seen_ms);
            *target = Some(Peer {
                id: source.device_id,
                rssi_dbm: source.rssi_dbm.get(),
                age_ms: age_ms.min(u32::MAX as u64) as u32,
                expires_in_ms: self
                    .peer_timeout_ms
                    .saturating_sub(age_ms)
                    .min(u32::MAX as u64) as u32,
            });
        }

        if self.status != Status::Fault && self.status != Status::Starting {
            self.status = if peer_count == 0 {
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
            tx_queue_full: 0,
            rx_queue_full: 0,
        }
    }
}
