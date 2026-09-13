//! Versioned ESP-NOW wire format.
//!
//! Beacons and application frames share a fixed header so firmware revisions
//! can tell each other apart. Postcard is used only for the application payload.
//!
//! ```text
//! offset  0..4   magic "HNHN"
//!         4      protocol version
//!         5      kind: beacon or application
//!         6      flags (application only)
//!         7      reserved, zero
//!         8..14  sender device id
//! beacon: 14..18 sequence, 18..22 uptime ms, zero-padded to 32 bytes
//! app:    14..20 recipient id or zero, 20..22 payload length, 22.. payload
//! ```

use core::{fmt, ops::Range};

pub(super) const BEACON_PACKET_BYTES: usize = 32;
pub(super) const MAX_RADIO_PACKET_BYTES: usize = 250;
/// Largest serialized application message that fits one radio frame.
pub const MAX_PAYLOAD: usize = 228;
pub(super) const PROTOCOL_VERSION: u8 = 2;

const MAGIC: [u8; 4] = *b"HNHN";
const MAGIC_FIELD: Range<usize> = 0..4;
const VERSION_OFFSET: usize = 4;
const KIND_OFFSET: usize = 5;
const FLAGS_OFFSET: usize = 6;
const SENDER_FIELD: Range<usize> = 8..14;
const BEACON_SEQUENCE_FIELD: Range<usize> = 14..18;
const BEACON_UPTIME_FIELD: Range<usize> = 18..22;
const RECIPIENT_FIELD: Range<usize> = 14..20;
const PAYLOAD_LEN_FIELD: Range<usize> = 20..22;
const APPLICATION_HEADER_BYTES: usize = 22;

const KIND_BEACON: u8 = 1;
const KIND_APPLICATION: u8 = 2;
const FLAG_RECIPIENT: u8 = 1 << 0;

const _: () = assert!(APPLICATION_HEADER_BYTES + MAX_PAYLOAD == MAX_RADIO_PACKET_BYTES);

/// Stable identity of a device, derived from its factory MAC address.
///
/// An all-zero identifier is invalid and cannot be represented.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DeviceId([u8; 6]);

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

/// ESP-NOW station address of a peer. Used for routing only, never shown to
/// applications, which see the [`DeviceId`] instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct MacAddress(pub(super) [u8; 6]);

impl fmt::Display for MacAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_mac(f, &self.0)
    }
}

fn write_mac(f: &mut fmt::Formatter<'_>, bytes: &[u8; 6]) -> fmt::Result {
    write!(
        f,
        "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5]
    )
}

/// Received signal strength in dBm.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct RssiDbm(pub(super) i8);

impl RssiDbm {
    pub(super) fn from_dbm(dbm: i32) -> Self {
        Self(i8::try_from(dbm).unwrap_or(i8::MIN))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct BeaconPacket {
    pub(super) device_id: DeviceId,
    pub(super) sequence: u32,
    pub(super) uptime_ms: u32,
}

impl BeaconPacket {
    pub(super) fn encode(self) -> [u8; BEACON_PACKET_BYTES] {
        let mut out = [0u8; BEACON_PACKET_BYTES];
        out[MAGIC_FIELD].copy_from_slice(&MAGIC);
        out[VERSION_OFFSET] = PROTOCOL_VERSION;
        out[KIND_OFFSET] = KIND_BEACON;
        out[SENDER_FIELD].copy_from_slice(&self.device_id.0);
        out[BEACON_SEQUENCE_FIELD].copy_from_slice(&self.sequence.to_le_bytes());
        out[BEACON_UPTIME_FIELD].copy_from_slice(&self.uptime_ms.to_le_bytes());
        out
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ApplicationPacket<'a> {
    pub(super) sender: DeviceId,
    pub(super) recipient: Option<DeviceId>,
    pub(super) payload: &'a [u8],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DecodedFrame<'a> {
    Beacon(BeaconPacket),
    Application(ApplicationPacket<'a>),
}

/// Encode an application frame into `out`. Returns the frame length, or `None`
/// when the payload is too large.
pub(super) fn encode_application(
    sender: DeviceId,
    recipient: Option<DeviceId>,
    payload: &[u8],
    out: &mut [u8; MAX_RADIO_PACKET_BYTES],
) -> Option<usize> {
    let payload_len = u16::try_from(payload.len()).ok()?;
    if payload.len() > MAX_PAYLOAD {
        return None;
    }

    out.fill(0);
    out[MAGIC_FIELD].copy_from_slice(&MAGIC);
    out[VERSION_OFFSET] = PROTOCOL_VERSION;
    out[KIND_OFFSET] = KIND_APPLICATION;
    out[SENDER_FIELD].copy_from_slice(&sender.0);
    if let Some(recipient) = recipient {
        out[FLAGS_OFFSET] = FLAG_RECIPIENT;
        out[RECIPIENT_FIELD].copy_from_slice(&recipient.0);
    }
    out[PAYLOAD_LEN_FIELD].copy_from_slice(&payload_len.to_le_bytes());
    out[APPLICATION_HEADER_BYTES..APPLICATION_HEADER_BYTES + payload.len()]
        .copy_from_slice(payload);
    Some(APPLICATION_HEADER_BYTES + payload.len())
}

/// Decode a received frame. Anything that is not exactly a frame of this
/// protocol version is rejected.
pub(super) fn decode_frame(bytes: &[u8]) -> Option<DecodedFrame<'_>> {
    if bytes.len() <= KIND_OFFSET
        || bytes[MAGIC_FIELD] != MAGIC
        || bytes[VERSION_OFFSET] != PROTOCOL_VERSION
    {
        return None;
    }

    match bytes[KIND_OFFSET] {
        KIND_BEACON => decode_beacon(bytes).map(DecodedFrame::Beacon),
        KIND_APPLICATION => decode_application(bytes).map(DecodedFrame::Application),
        _ => None,
    }
}

fn device_id(bytes: &[u8]) -> Option<DeviceId> {
    DeviceId::try_from(<[u8; 6]>::try_from(bytes).ok()?).ok()
}

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

    let payload_len = usize::from(u16::from_le_bytes(
        bytes[PAYLOAD_LEN_FIELD].try_into().ok()?,
    ));
    if payload_len > MAX_PAYLOAD || bytes.len() != APPLICATION_HEADER_BYTES + payload_len {
        return None;
    }

    Some(ApplicationPacket {
        sender,
        recipient,
        payload: &bytes[APPLICATION_HEADER_BYTES..],
    })
}
