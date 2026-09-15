//! Versioned ESP-NOW wire format.
//!
//! Beacons and application frames start with the same fixed header. It holds
//! a marker and the protocol version, so that boards with a different
//! firmware version ignore each other's frames. Only the application payload
//! uses `postcard` encoding.
//!
//! The frame layout, in bytes. Ranges include the start and exclude the end.
//! Numbers of more than one byte are little-endian (low byte first).
//!
//! ```text
//! offset  0..4   magic "HNHN"
//!         4      protocol version
//!         5      frame type: beacon or application
//!         6      flags (application only)
//!         7      reserved, zero
//!         8..14  sender device id
//! beacon: 14..18 sequence, 18..22 uptime ms, zero-padded to 32 bytes
//! app:    14..20 recipient id or zero, 20..24 message kind,
//!         24..26 payload length, 26.. payload
//! ```
//!
//! The message kind names the application type inside the payload (see
//! [`super::message::Message`]), so boards running different applications on
//! the same channel can ignore each other's messages.

use core::{fmt, ops::Range};

/// Length of every beacon frame, in bytes.
pub const BEACON_PACKET_BYTES: usize = 32;
/// Largest frame that ESP-NOW sends, in bytes.
pub const MAX_RADIO_PACKET_BYTES: usize = 250;
/// Largest encoded application message, in bytes, that fits into one radio
/// frame.
pub const MAX_PAYLOAD: usize = 224;
/// Version of this format. Boards only understand frames of their own
/// version, so firmware of an older layout is ignored instead of misread.
pub const PROTOCOL_VERSION: u8 = 3;

/// The first bytes of every frame. They mark the frame as a frame of this
/// protocol, among other ESP-NOW traffic.
const MAGIC: [u8; 4] = *b"HNHN";
/// Where [`MAGIC`] sits in the frame.
const MAGIC_FIELD: Range<usize> = 0..4;
/// Byte holding [`PROTOCOL_VERSION`].
const VERSION_OFFSET: usize = 4;
/// Byte holding the frame type: beacon or application.
const FRAME_TYPE_OFFSET: usize = 5;
/// Byte holding the application flags; zero in beacons.
const FLAGS_OFFSET: usize = 6;
/// The sender's [`DeviceId`], in both frame types.
const SENDER_FIELD: Range<usize> = 8..14;
/// Beacon sequence number, little-endian `u32`.
const BEACON_SEQUENCE_FIELD: Range<usize> = 14..18;
/// Beacon sender uptime in milliseconds, little-endian `u32`.
const BEACON_UPTIME_FIELD: Range<usize> = 18..22;
/// Recipient [`DeviceId`] of an application frame; all zero for a
/// broadcast.
const RECIPIENT_FIELD: Range<usize> = 14..20;
/// Message kind of an application frame, little-endian `u32`.
const MESSAGE_KIND_FIELD: Range<usize> = 20..24;
/// Payload length in bytes of an application frame, little-endian
/// `u16`.
const PAYLOAD_LEN_FIELD: Range<usize> = 24..26;
/// Size of the application frame header; the payload starts here.
const APPLICATION_HEADER_BYTES: usize = 26;

/// Frame type byte of a beacon.
const FRAME_TYPE_BEACON: u8 = 1;
/// Frame type byte of an application message.
const FRAME_TYPE_APPLICATION: u8 = 2;
/// Flag set when an application frame is addressed to one board; clear
/// for a broadcast.
const FLAG_RECIPIENT: u8 = 1 << 0;

// Checked at compile time: the header and the largest payload together fill
// one radio frame exactly.
const _: () = assert!(APPLICATION_HEADER_BYTES + MAX_PAYLOAD == MAX_RADIO_PACKET_BYTES);

/// The identity of a board. It comes from the factory MAC address of the
/// board, so it never changes. It is shown as `AA:BB:CC:DD:EE:FF`.
///
/// An identifier of all zeros is not valid, so a `DeviceId` is never all
/// zeros.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DeviceId([u8; 6]);

/// The six bytes of a device id were all zero.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvalidDeviceId;

impl TryFrom<[u8; 6]> for DeviceId {
    type Error = InvalidDeviceId;

    fn try_from(bytes: [u8; 6]) -> Result<Self, Self::Error> {
        if bytes == [0; 6] {
            Err(InvalidDeviceId)
        } else {
            Ok(Self(bytes))
        }
    }
}

impl fmt::Display for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_mac(f, &self.0)
    }
}

/// The radio (MAC) address of a board, where ESP-NOW sends frames. Only the
/// network code uses it. Applications see the [`DeviceId`] instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MacAddress(pub [u8; 6]);

impl fmt::Display for MacAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_mac(f, &self.0)
    }
}

/// Write six bytes in the usual MAC address form, `AA:BB:CC:DD:EE:FF`.
fn write_mac(f: &mut fmt::Formatter<'_>, bytes: &[u8; 6]) -> fmt::Result {
    write!(
        f,
        "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5]
    )
}

/// "I am here": sent by every board to all boards four times per second.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BeaconPacket {
    /// The board sending the beacon.
    pub device_id: DeviceId,
    /// Counts up with every beacon of this board.
    pub sequence: u32,
    /// How long the board has been running, in milliseconds. It wraps
    /// around to 0 after about 49 days.
    pub uptime_ms: u32,
}

impl BeaconPacket {
    /// The beacon as a frame, ready to send.
    pub fn encode(self) -> [u8; BEACON_PACKET_BYTES] {
        let mut out = [0u8; BEACON_PACKET_BYTES];
        out[MAGIC_FIELD].copy_from_slice(&MAGIC);
        out[VERSION_OFFSET] = PROTOCOL_VERSION;
        out[FRAME_TYPE_OFFSET] = FRAME_TYPE_BEACON;
        out[SENDER_FIELD].copy_from_slice(&self.device_id.0);
        out[BEACON_SEQUENCE_FIELD].copy_from_slice(&self.sequence.to_le_bytes());
        out[BEACON_UPTIME_FIELD].copy_from_slice(&self.uptime_ms.to_le_bytes());
        out
    }
}

/// A message from one application to another, as it travels on the radio.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ApplicationPacket<'a> {
    /// The board that sent the message.
    pub sender: DeviceId,
    /// The board it is for, or `None` for everyone.
    pub recipient: Option<DeviceId>,
    /// Which application message type the payload holds.
    pub kind: u32,
    /// The `postcard` encoding of the message.
    pub payload: &'a [u8],
}

/// A received frame, after [`decode_frame`] has checked it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodedFrame<'a> {
    /// A beacon: the sender is in range.
    Beacon(BeaconPacket),
    /// An application message; `payload` borrows from the received bytes.
    Application(ApplicationPacket<'a>),
}

/// Encode an application frame into `out`. Returns the frame length in
/// bytes, or `None` when the payload is larger than [`MAX_PAYLOAD`].
pub fn encode_application(
    packet: ApplicationPacket<'_>,
    out: &mut [u8; MAX_RADIO_PACKET_BYTES],
) -> Option<usize> {
    let payload = packet.payload;
    if payload.len() > MAX_PAYLOAD {
        return None;
    }
    let payload_len = u16::try_from(payload.len()).ok()?;

    out.fill(0);
    out[MAGIC_FIELD].copy_from_slice(&MAGIC);
    out[VERSION_OFFSET] = PROTOCOL_VERSION;
    out[FRAME_TYPE_OFFSET] = FRAME_TYPE_APPLICATION;
    out[SENDER_FIELD].copy_from_slice(&packet.sender.0);
    if let Some(recipient) = packet.recipient {
        out[FLAGS_OFFSET] = FLAG_RECIPIENT;
        out[RECIPIENT_FIELD].copy_from_slice(&recipient.0);
    }
    out[MESSAGE_KIND_FIELD].copy_from_slice(&packet.kind.to_le_bytes());
    out[PAYLOAD_LEN_FIELD].copy_from_slice(&payload_len.to_le_bytes());
    out[APPLICATION_HEADER_BYTES..APPLICATION_HEADER_BYTES + payload.len()]
        .copy_from_slice(payload);
    Some(APPLICATION_HEADER_BYTES + payload.len())
}

/// Decode a received frame. Returns `None` for anything that is not exactly
/// a valid frame of this protocol version.
pub fn decode_frame(bytes: &[u8]) -> Option<DecodedFrame<'_>> {
    if bytes.len() <= FRAME_TYPE_OFFSET
        || bytes[MAGIC_FIELD] != MAGIC
        || bytes[VERSION_OFFSET] != PROTOCOL_VERSION
    {
        return None;
    }

    match bytes[FRAME_TYPE_OFFSET] {
        FRAME_TYPE_BEACON => decode_beacon(bytes).map(DecodedFrame::Beacon),
        FRAME_TYPE_APPLICATION => decode_application(bytes).map(DecodedFrame::Application),
        _ => None,
    }
}

/// A [`DeviceId`] from a six-byte slice; `None` for another length or all
/// zeros.
fn device_id(bytes: &[u8]) -> Option<DeviceId> {
    DeviceId::try_from(<[u8; 6]>::try_from(bytes).ok()?).ok()
}

/// Decode a frame whose header says beacon. Checks the length and the
/// sender id.
fn decode_beacon(bytes: &[u8]) -> Option<BeaconPacket> {
    if bytes.len() != BEACON_PACKET_BYTES {
        return None;
    }

    Some(BeaconPacket {
        device_id: device_id(&bytes[SENDER_FIELD])?,
        sequence: u32::from_le_bytes(bytes[BEACON_SEQUENCE_FIELD].try_into().ok()?),
        uptime_ms: u32::from_le_bytes(bytes[BEACON_UPTIME_FIELD].try_into().ok()?),
    })
}

/// Decode a frame whose header says application message.
///
/// Rejects a frame that is shorter than the header or longer than a radio
/// frame, has unknown flags or an all-zero sender id, or has a recipient
/// field that does not match the recipient flag. Also rejects a frame whose
/// length does not match the payload length field.
fn decode_application(bytes: &[u8]) -> Option<ApplicationPacket<'_>> {
    if bytes.len() < APPLICATION_HEADER_BYTES || bytes.len() > MAX_RADIO_PACKET_BYTES {
        return None;
    }
    let flags = bytes[FLAGS_OFFSET];
    if flags & !FLAG_RECIPIENT != 0 {
        return None;
    }

    let sender = device_id(&bytes[SENDER_FIELD])?;
    let recipient = if flags & FLAG_RECIPIENT != 0 {
        Some(device_id(&bytes[RECIPIENT_FIELD])?)
    } else if bytes[RECIPIENT_FIELD] == [0; 6] {
        None
    } else {
        return None;
    };

    let kind = u32::from_le_bytes(bytes[MESSAGE_KIND_FIELD].try_into().ok()?);
    let payload_len = usize::from(u16::from_le_bytes(
        bytes[PAYLOAD_LEN_FIELD].try_into().ok()?,
    ));
    if payload_len > MAX_PAYLOAD || bytes.len() != APPLICATION_HEADER_BYTES + payload_len {
        return None;
    }

    Some(ApplicationPacket {
        sender,
        recipient,
        kind,
        payload: &bytes[APPLICATION_HEADER_BYTES..],
    })
}

// Unit tests live next to the code they check. Scenario tests that combine
// several modules are in `crates/core/tests/`.
#[cfg(test)]
mod tests {
    use super::*;

    fn id(last: u8) -> DeviceId {
        DeviceId::try_from([1, 2, 3, 4, 5, last]).unwrap()
    }

    #[test]
    fn beacon_roundtrips() {
        let beacon = BeaconPacket {
            device_id: id(6),
            sequence: 7,
            uptime_ms: 1234,
        };
        assert_eq!(
            decode_frame(&beacon.encode()),
            Some(DecodedFrame::Beacon(beacon))
        );
    }

    #[test]
    fn application_frame_roundtrips() {
        let packet = ApplicationPacket {
            sender: id(6),
            recipient: Some(id(7)),
            kind: 0xDEAD_BEEF,
            payload: &[9, 8, 7, 6],
        };
        let mut frame = [0u8; MAX_RADIO_PACKET_BYTES];
        let len = encode_application(packet, &mut frame).expect("payload fits");
        assert_eq!(
            decode_frame(&frame[..len]),
            Some(DecodedFrame::Application(packet))
        );
    }

    #[test]
    fn oversized_payload_is_refused() {
        let packet = ApplicationPacket {
            sender: id(6),
            recipient: None,
            kind: 1,
            payload: &[0; MAX_PAYLOAD + 1],
        };
        let mut frame = [0u8; MAX_RADIO_PACKET_BYTES];
        assert_eq!(encode_application(packet, &mut frame), None);
    }

    #[test]
    fn corrupted_frames_are_rejected() {
        let packet = ApplicationPacket {
            sender: id(6),
            recipient: Some(id(7)),
            kind: 1,
            payload: &[9, 8, 7, 6],
        };
        let mut frame = [0u8; MAX_RADIO_PACKET_BYTES];
        let len = encode_application(packet, &mut frame).unwrap();

        // Each closure damages one copy of the good frame.
        let damaged: [fn(&mut [u8]); 5] = [
            |frame| frame[MAGIC_FIELD.start] ^= 0xFF,
            |frame| frame[VERSION_OFFSET] += 1,
            |frame| frame[PAYLOAD_LEN_FIELD].copy_from_slice(&99u16.to_le_bytes()),
            |frame| frame[FLAGS_OFFSET] = 0x80,
            // A recipient without the flag is not a broadcast either.
            |frame| frame[FLAGS_OFFSET] = 0,
        ];
        for damage in damaged {
            let mut copy = frame;
            damage(&mut copy[..len]);
            assert_eq!(decode_frame(&copy[..len]), None);
        }

        assert_eq!(decode_frame(&frame[..len - 1]), None);
        assert_eq!(decode_frame(b"HNH"), None);
    }
}
