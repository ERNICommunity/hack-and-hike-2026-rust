#[path = "../../../src/capabilities/network/protocol.rs"]
mod protocol;

pub(crate) use protocol::{DeviceId, MAX_PAYLOAD};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Channel(u8);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RssiDbm(i8);

impl RssiDbm {
    const fn get(self) -> i8 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct MacAddress([u8; 6]);

impl MacAddress {
    const fn new(bytes: [u8; 6]) -> Self {
        Self(bytes)
    }
}

const MAX_PEERS: usize = 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Status {
    Starting,
    Ready,
    PeerPresent,
    Fault,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Peer {
    id: DeviceId,
    rssi_dbm: i8,
    age_ms: u32,
    expires_in_ms: u32,
}

#[derive(Clone, Copy, Debug)]
struct Snapshot {
    revision: u32,
    status: Status,
    local_id: DeviceId,
    channel: Channel,
    peers: [Option<Peer>; MAX_PEERS],
    tx_packets: u32,
    rx_packets: u32,
    tx_errors: u32,
    rx_invalid: u32,
    peer_evictions: u32,
    tx_queue_full: u32,
    rx_queue_full: u32,
}

#[path = "../../../src/capabilities/network/message.rs"]
mod message;
#[path = "../../../src/capabilities/network/state.rs"]
mod state;

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};

    use super::{
        Channel, DeviceId, MAX_PAYLOAD, MacAddress, RssiDbm,
        message::{IncomingMessage, SendError, serialize_payload},
        protocol::{self, DecodedFrame},
        state::NetworkState,
    };

    fn id(bytes: [u8; 6]) -> DeviceId {
        DeviceId::try_from(bytes).unwrap()
    }

    #[test]
    fn explicit_envelopes_roundtrip_and_reject_malformed_frames() {
        let sender = id([1, 2, 3, 4, 5, 6]);
        let recipient = id([6, 5, 4, 3, 2, 1]);
        let capabilities = protocol::Capabilities::from_enabled(true, false, true);

        let beacon = protocol::BeaconPacket::new(sender, 7, 1234, capabilities).encode();
        match protocol::decode_frame(&beacon) {
            Some(DecodedFrame::Beacon(decoded)) => {
                assert_eq!(decoded.device_id, sender);
                assert_eq!(decoded.sequence, 7);
                assert_eq!(decoded.uptime_ms, 1234);
                assert_eq!(decoded.capabilities, capabilities);
                assert_eq!(decoded.capabilities.bits(), 0b101);
            }
            other => panic!("unexpected beacon decode: {other:?}"),
        }

        let payload = [9u8, 8, 7, 6];
        let mut encoded = [0u8; protocol::MAX_RADIO_PACKET_BYTES];
        let len = protocol::encode_application(sender, Some(recipient), &payload, &mut encoded)
            .expect("payload fits");
        match protocol::decode_frame(&encoded[..len]) {
            Some(DecodedFrame::Application(decoded)) => {
                assert_eq!(decoded.sender, sender);
                assert_eq!(decoded.recipient, Some(recipient));
                assert_eq!(decoded.payload, payload);
            }
            other => panic!("unexpected application decode: {other:?}"),
        }

        let mut bad_magic = encoded;
        bad_magic[0] ^= 0xff;
        assert!(protocol::decode_frame(&bad_magic[..len]).is_none());

        let mut bad_length = encoded;
        bad_length[20..22].copy_from_slice(&99u16.to_le_bytes());
        assert!(protocol::decode_frame(&bad_length[..len]).is_none());

        let mut bad_flags = encoded;
        bad_flags[6] = 0x80;
        assert!(protocol::decode_frame(&bad_flags[..len]).is_none());
    }

    #[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
    enum DemoMessage {
        Ping { sequence: u32 },
        Pong { sequence: u32 },
    }

    #[derive(Serialize)]
    struct Huge<'a> {
        bytes: &'a [u8],
    }

    #[test]
    fn postcard_payloads_roundtrip_and_oversize_is_explicit() {
        let sender = id([1, 1, 1, 1, 1, 1]);
        let value = DemoMessage::Ping { sequence: 42 };
        let payload = serialize_payload(&value).expect("demo message must fit");
        let incoming = IncomingMessage::from_bytes(sender, None, payload.as_slice()).unwrap();
        assert_eq!(incoming.decode::<DemoMessage>().unwrap(), value);

        let bytes = [0u8; MAX_PAYLOAD];
        assert_eq!(
            serialize_payload(&Huge { bytes: &bytes }),
            Err(SendError::MessageTooLarge)
        );
    }

    #[test]
    fn peer_state_tracks_private_routes_age_and_expiry() {
        let local = id([1, 2, 3, 4, 5, 6]);
        let peer = id([6, 5, 4, 3, 2, 1]);
        let mac = MacAddress::new([10, 11, 12, 13, 14, 15]);
        let mut state = NetworkState::new(local, Channel(6), 500);
        state.mark_ready();

        let outcome = state.record_receive(peer, mac, RssiDbm(-42), 1000);
        assert!(outcome.is_new);
        assert!(!outcome.evicted);
        assert_eq!(state.route_for(peer), Some(mac));

        let snapshot = state.snapshot(1200);
        let peer_snapshot = snapshot.peers.iter().flatten().next().unwrap();
        assert_eq!(peer_snapshot.id, peer);
        assert_eq!(peer_snapshot.rssi_dbm, -42);
        assert_eq!(peer_snapshot.age_ms, 200);
        assert_eq!(peer_snapshot.expires_in_ms, 300);

        let expired = state.snapshot(1501);
        assert!(expired.peers.iter().all(Option::is_none));
        assert_eq!(state.route_for(peer), None);
    }
}
