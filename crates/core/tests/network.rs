//! Wire protocol, message and peer-table behaviour.

use serde::{Deserialize, Serialize};

use hack_and_hike_core::network::{
    message::{IncomingMessage, SendError, serialize_payload},
    protocol::{self, DecodedFrame, DeviceId, MAX_PAYLOAD, MacAddress, RssiDbm},
    state::{Channel, Heard, MAX_PEERS, NetworkState, QueueCounters, Status},
};

fn id(bytes: [u8; 6]) -> DeviceId {
    DeviceId::try_from(bytes).unwrap()
}

/// A beacon, which carries the sender's uptime.
fn beacon_from(device_id: DeviceId, mac: MacAddress, uptime_ms: u32) -> Heard {
    Heard {
        device_id,
        mac,
        rssi_dbm: RssiDbm(-42),
        uptime_ms: Some(uptime_ms),
    }
}

/// An application message, which says nothing about uptime.
fn message_from(device_id: DeviceId, mac: MacAddress) -> Heard {
    Heard {
        device_id,
        mac,
        rssi_dbm: RssiDbm(-50),
        uptime_ms: None,
    }
}

fn state() -> NetworkState {
    let mut state = NetworkState::new(id([1, 2, 3, 4, 5, 6]), Channel::new(6), 500);
    state.mark_ready();
    state
}

#[test]
fn envelopes_roundtrip_and_reject_malformed_frames() {
    let sender = id([1, 2, 3, 4, 5, 6]);
    let recipient = id([6, 5, 4, 3, 2, 1]);

    let beacon = protocol::BeaconPacket {
        device_id: sender,
        sequence: 7,
        uptime_ms: 1234,
    };
    assert_eq!(
        protocol::decode_frame(&beacon.encode()),
        Some(DecodedFrame::Beacon(beacon))
    );

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

    let mut stray_recipient = encoded;
    stray_recipient[6] = 0;
    assert!(protocol::decode_frame(&stray_recipient[..len]).is_none());
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
    let incoming = IncomingMessage::from_bytes(sender, payload.as_slice()).unwrap();
    assert_eq!(incoming.decode::<DemoMessage>().unwrap(), value);

    let bytes = [0u8; MAX_PAYLOAD];
    assert_eq!(
        serialize_payload(&Huge { bytes: &bytes }),
        Err(SendError::MessageTooLarge)
    );
}

#[test]
fn peer_state_tracks_routes_age_and_expiry() {
    let peer = id([6, 5, 4, 3, 2, 1]);
    let mac = MacAddress([10, 11, 12, 13, 14, 15]);
    let mut state = state();

    let received = state.record_receive(beacon_from(peer, mac, 60_000), 1000);
    assert!(received.is_new);
    assert!(received.evicted.is_none());
    assert_eq!(state.route_for(peer), Some(mac));

    let snapshot = state.snapshot(1200, QueueCounters::default());
    assert_eq!(snapshot.status, Status::PeerPresent);
    let peer_snapshot = snapshot.peers().next().unwrap();
    assert_eq!(peer_snapshot.id, peer);
    assert_eq!(peer_snapshot.rssi_dbm, -42);
    assert_eq!(peer_snapshot.age_ms, 200);
    assert_eq!(peer_snapshot.expires_in_ms, 300);
    // Uptime keeps counting between beacons.
    assert_eq!(peer_snapshot.uptime_ms, Some(60_200));

    // An application message refreshes the peer but knows no uptime.
    state.record_receive(message_from(peer, mac), 1300);
    let peer_snapshot = *state
        .snapshot(1300, QueueCounters::default())
        .peers()
        .next()
        .unwrap();
    assert_eq!(peer_snapshot.rssi_dbm, -50);
    assert_eq!(peer_snapshot.uptime_ms, Some(60_000));

    // The peer was last heard at 1300 and the timeout is 500 ms.
    assert!(state.expire_peers(1800).is_empty());
    let expired = state.expire_peers(1801);
    assert_eq!(expired.as_slice(), &[mac]);
    assert_eq!(state.route_for(peer), None);
    let snapshot = state.snapshot(1801, QueueCounters::default());
    assert_eq!(snapshot.status, Status::Ready);
    assert_eq!(snapshot.peer_count(), 0);
}

#[test]
fn full_peer_table_evicts_the_longest_unseen_peer() {
    let mut state = state();
    let peer_id = |n: u8| id([0x10, 0, 0, 0, 0, n]);
    let peer_mac = |n: u8| MacAddress([0x20, 0, 0, 0, 0, n]);

    for n in 0..MAX_PEERS {
        let n = u8::try_from(n).unwrap();
        let heard = message_from(peer_id(n), peer_mac(n));
        let received = state.record_receive(heard, 1000 + u64::from(n));
        assert!(received.is_new);
        assert!(received.evicted.is_none());
    }

    let newcomer = u8::try_from(MAX_PEERS).unwrap();
    let received = state.record_receive(message_from(peer_id(newcomer), peer_mac(newcomer)), 2000);
    assert!(received.is_new);
    assert_eq!(received.evicted, Some(peer_mac(0)));
    assert_eq!(state.route_for(peer_id(0)), None);
    assert_eq!(state.route_for(peer_id(newcomer)), Some(peer_mac(newcomer)));
    assert_eq!(
        state
            .snapshot(2000, QueueCounters::default())
            .peer_evictions,
        1
    );
}
