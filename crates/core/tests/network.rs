//! Typed messages and peer-table behaviour, as the firmware uses them.
//!
//! The wire format itself is unit-tested in `src/network/protocol.rs`.

use serde::{Deserialize, Serialize};

use hack_and_hike_core::network::{
    message::{DecodeError, IncomingMessage, Message, OutgoingMessage, SendError},
    protocol::{DeviceId, MAX_PAYLOAD, MacAddress},
    state::{Heard, MAX_PEERS, NetworkState, QueueCounters, RadioChannel, Status},
};

fn id(bytes: [u8; 6]) -> DeviceId {
    DeviceId::try_from(bytes).unwrap()
}

/// Send `value` through the same steps the radio uses and hand it back as
/// the receiving board would see it.
fn transmit<T: Message>(value: &T) -> IncomingMessage {
    let outgoing = OutgoingMessage::new(None, value).expect("message fits a frame");
    IncomingMessage::from_bytes(id([1, 1, 1, 1, 1, 1]), outgoing.kind, &outgoing.payload).unwrap()
}

// The demo's ping-pong messages...
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
enum DemoMessage {
    Ping { sequence: u32 },
    Pong { sequence: u32 },
}

impl Message for DemoMessage {
    const NAME: &'static str = "demo.ping-pong";
}

// ...and Color Ping's, which happen to encode to the same bytes.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
enum Color {
    Red,
    Green,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
struct ColorPing {
    color: Color,
}

impl Message for ColorPing {
    const NAME: &'static str = "color-ping";
}

#[test]
fn messages_roundtrip() {
    let sent = DemoMessage::Ping { sequence: 42 };
    let received = transmit(&sent);
    assert!(received.is::<DemoMessage>());
    assert_eq!(received.decode::<DemoMessage>(), Ok(sent));
}

#[test]
fn messages_of_another_application_are_not_decoded() {
    // `Ping { sequence: 1 }` and `ColorPing { color: Green }` are both the
    // bytes `[0, 1]`; only the kind tells them apart.
    let received = transmit(&DemoMessage::Ping { sequence: 1 });
    assert!(!received.is::<ColorPing>());
    assert_eq!(received.decode::<ColorPing>(), Err(DecodeError::WrongKind));
}

#[test]
fn malformed_payloads_are_rejected() {
    let sender = id([1, 1, 1, 1, 1, 1]);
    // A truncated varint.
    let truncated = IncomingMessage::from_bytes(sender, DemoMessage::KIND, &[0, 0x80]).unwrap();
    assert_eq!(
        truncated.decode::<DemoMessage>(),
        Err(DecodeError::Malformed)
    );
    // Trailing bytes the type does not account for.
    let too_long = IncomingMessage::from_bytes(sender, DemoMessage::KIND, &[0, 1, 99]).unwrap();
    assert_eq!(
        too_long.decode::<DemoMessage>(),
        Err(DecodeError::Malformed)
    );
}

#[test]
fn oversized_messages_are_refused_when_queued() {
    // 256 bytes; serde derives array support up to 32 elements, hence 8 x 32.
    #[derive(Serialize, Deserialize)]
    struct Huge {
        bytes: [[u8; 32]; 8],
    }
    impl Message for Huge {
        const NAME: &'static str = "huge";
    }

    let huge = Huge {
        bytes: [[0; 32]; 8],
    };
    assert!(8 * 32 > MAX_PAYLOAD);
    assert!(matches!(
        OutgoingMessage::new(None, &huge),
        Err(SendError::MessageTooLarge)
    ));
}

#[test]
fn message_kinds_come_from_their_names() {
    assert_ne!(DemoMessage::KIND, ColorPing::KIND);
    assert_eq!(
        DemoMessage::KIND,
        hack_and_hike_core::network::message::message_kind("demo.ping-pong")
    );
}

/// A beacon, which carries the sender's uptime.
fn beacon_from(device_id: DeviceId, mac: MacAddress, uptime_ms: u32) -> Heard {
    Heard {
        device_id,
        mac,
        rssi_dbm: -42,
        uptime_ms: Some(uptime_ms),
    }
}

/// An application message, which says nothing about uptime.
fn message_from(device_id: DeviceId, mac: MacAddress) -> Heard {
    Heard {
        device_id,
        mac,
        rssi_dbm: -50,
        uptime_ms: None,
    }
}

const PEER_TIMEOUT_MS: u64 = 500;

fn ready_state() -> NetworkState {
    let mut state = NetworkState::new(
        id([1, 2, 3, 4, 5, 6]),
        RadioChannel::new(6),
        PEER_TIMEOUT_MS,
    );
    state.mark_ready();
    state
}

const PEER: [u8; 6] = [6, 5, 4, 3, 2, 1];
const PEER_MAC: MacAddress = MacAddress([10, 11, 12, 13, 14, 15]);

#[test]
fn a_heard_beacon_becomes_a_peer_with_a_route() {
    let mut state = ready_state();

    let received = state.record_receive(beacon_from(id(PEER), PEER_MAC, 60_000), 1000);
    assert!(received.is_new);
    assert!(received.evicted.is_none());
    assert_eq!(state.route_for(id(PEER)), Some(PEER_MAC));

    let snapshot = state.snapshot(1200, QueueCounters::default());
    assert_eq!(snapshot.status, Status::PeerPresent);
    let peer = snapshot.peers().next().unwrap();
    assert_eq!(peer.id, id(PEER));
    assert_eq!(peer.rssi_dbm, -42);
    assert_eq!(peer.age_ms, 200);
    assert_eq!(peer.expires_in_ms, 300);
    // Uptime keeps counting between beacons.
    assert_eq!(peer.uptime_ms, Some(60_200));
}

#[test]
fn an_application_message_refreshes_a_peer_but_keeps_its_uptime() {
    let mut state = ready_state();
    state.record_receive(beacon_from(id(PEER), PEER_MAC, 60_000), 1000);

    state.record_receive(message_from(id(PEER), PEER_MAC), 1300);
    let snapshot = state.snapshot(1300, QueueCounters::default());
    let peer = snapshot.peers().next().unwrap();
    assert_eq!(peer.rssi_dbm, -50);
    assert_eq!(peer.uptime_ms, Some(60_000 + 300));
}

#[test]
fn silent_peers_expire_after_the_timeout() {
    let mut state = ready_state();
    state.record_receive(beacon_from(id(PEER), PEER_MAC, 0), 1300);

    assert!(state.expire_peers(1300 + PEER_TIMEOUT_MS).is_empty());
    let expired = state.expire_peers(1300 + PEER_TIMEOUT_MS + 1);
    assert_eq!(expired.as_slice(), &[PEER_MAC]);
    assert_eq!(state.route_for(id(PEER)), None);

    let snapshot = state.snapshot(2000, QueueCounters::default());
    assert_eq!(snapshot.status, Status::Ready);
    assert_eq!(snapshot.peer_count(), 0);
}

#[test]
fn a_full_peer_table_evicts_the_longest_unseen_peer() {
    let mut state = ready_state();
    let peer_id = |n: u8| id([0x10, 0, 0, 0, 0, n]);
    let peer_mac = |n: u8| MacAddress([0x20, 0, 0, 0, 0, n]);

    for n in 0..MAX_PEERS {
        let n = u8::try_from(n).unwrap();
        let received =
            state.record_receive(message_from(peer_id(n), peer_mac(n)), 1000 + u64::from(n));
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

#[test]
fn channel_numbers_are_validated() {
    assert_eq!(RadioChannel::try_from(6).map(RadioChannel::number), Ok(6));
    assert!(RadioChannel::try_from(0).is_err());
    assert!(RadioChannel::try_from(15).is_err());
}
