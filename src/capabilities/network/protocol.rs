//! Explicit, versioned Hack and Hike ESP-NOW wire protocol.
//!
//! Discovery remains a fixed binary envelope so firmware revisions can reason
//! about transport compatibility independently of application schemas. Postcard
//! is used only for application payload bytes at the public capability boundary.

use core::fmt;

pub(super) const BEACON_PACKET_BYTES: usize = 32;
pub(super) const MAX_RADIO_PACKET_BYTES: usize = 250;
pub(crate) const MAX_PAYLOAD: usize = 228;
pub(super) const PROTOCOL_VERSION: u8 = 1;

const MAGIC: [u8; 4] = *b"HNHN";
const KIND_BEACON: u8 = 1;
const KIND_APPLICATION: u8 = 2;
const FLAG_RECIPIENT: u8 = 1 << 0;
const APPLICATION_HEADER_BYTES: usize = 22;
const _: () = assert!(APPLICATION_HEADER_BYTES + MAX_PAYLOAD == MAX_RADIO_PACKET_BYTES);

/// Stable physical-device identity derived from the factory eFuse MAC.
///
/// An all-zero identifier is invalid and cannot be represented as `DeviceId`.
/// MAC addresses used by ESP-NOW routing stay private to the capability.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct DeviceId([u8; 6]);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct InvalidDeviceId;

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
        write!(
            f,
            "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
            self.0[0], self.0[1], self.0[2], self.0[3], self.0[4], self.0[5]
        )
    }
}

/// Capability bits carried by discovery beacons.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Capabilities(u32);

impl Capabilities {
    pub(super) const IMU: Self = Self(1 << 0);
    pub(super) const AUDIO: Self = Self(1 << 1);
    pub(super) const DISPLAY: Self = Self(1 << 2);

    pub(super) const fn from_enabled(imu: bool, audio: bool, display: bool) -> Self {
        let mut bits = 0;
        if imu {
            bits |= Self::IMU.0;
        }
        if audio {
            bits |= Self::AUDIO.0;
        }
        if display {
            bits |= Self::DISPLAY.0;
        }
        Self(bits)
    }

    pub(super) const fn bits(self) -> u32 {
        self.0
    }

    const fn from_bits(bits: u32) -> Self {
        Self(bits)
    }
}

impl fmt::UpperHex for Capabilities {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::UpperHex::fmt(&self.0, f)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct BeaconPacket {
    pub(super) device_id: DeviceId,
    pub(super) sequence: u32,
    pub(super) uptime_ms: u32,
    pub(super) capabilities: Capabilities,
}

impl BeaconPacket {
    pub(super) const fn new(
        device_id: DeviceId,
        sequence: u32,
        uptime_ms: u32,
        capabilities: Capabilities,
    ) -> Self {
        Self {
            device_id,
            sequence,
            uptime_ms,
            capabilities,
        }
    }

    pub(super) fn encode(self) -> [u8; BEACON_PACKET_BYTES] {
        let mut out = [0u8; BEACON_PACKET_BYTES];
        out[0..4].copy_from_slice(&MAGIC);
        out[4] = PROTOCOL_VERSION;
        out[5] = KIND_BEACON;
        out[8..14].copy_from_slice(&self.device_id.0);
        out[14..18].copy_from_slice(&self.sequence.to_le_bytes());
        out[18..22].copy_from_slice(&self.uptime_ms.to_le_bytes());
        out[22..26].copy_from_slice(&self.capabilities.bits().to_le_bytes());
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

pub(super) fn encode_application(
    sender: DeviceId,
    recipient: Option<DeviceId>,
    payload: &[u8],
    out: &mut [u8; MAX_RADIO_PACKET_BYTES],
) -> Option<usize> {
    if payload.len() > MAX_PAYLOAD {
        return None;
    }

    out.fill(0);
    out[0..4].copy_from_slice(&MAGIC);
    out[4] = PROTOCOL_VERSION;
    out[5] = KIND_APPLICATION;
    out[6] = if recipient.is_some() { FLAG_RECIPIENT } else { 0 };
    out[8..14].copy_from_slice(&sender.0);
    if let Some(recipient) = recipient {
        out[14..20].copy_from_slice(&recipient.0);
    }
    out[20..22].copy_from_slice(&(payload.len() as u16).to_le_bytes());
    out[APPLICATION_HEADER_BYTES..APPLICATION_HEADER_BYTES + payload.len()]
        .copy_from_slice(payload);
    Some(APPLICATION_HEADER_BYTES + payload.len())
}

pub(super) fn decode_frame(bytes: &[u8]) -> Option<DecodedFrame<'_>> {
    if bytes.len() < 6 || bytes[0..4] != MAGIC || bytes[4] != PROTOCOL_VERSION {
        return None;
    }

    match bytes[5] {
        KIND_BEACON => decode_beacon(bytes).map(DecodedFrame::Beacon),
        KIND_APPLICATION => decode_application(bytes).map(DecodedFrame::Application),
        _ => None,
    }
}

fn decode_beacon(bytes: &[u8]) -> Option<BeaconPacket> {
    if bytes.len() != BEACON_PACKET_BYTES {
        return None;
    }

    let mut id = [0u8; 6];
    id.copy_from_slice(&bytes[8..14]);

    Some(BeaconPacket {
        device_id: DeviceId::try_from(id).ok()?,
        sequence: u32::from_le_bytes(bytes[14..18].try_into().ok()?),
        uptime_ms: u32::from_le_bytes(bytes[18..22].try_into().ok()?),
        capabilities: Capabilities::from_bits(u32::from_le_bytes(
            bytes[22..26].try_into().ok()?,
        )),
    })
}

fn decode_application(bytes: &[u8]) -> Option<ApplicationPacket<'_>> {
    if bytes.len() < APPLICATION_HEADER_BYTES || bytes.len() > MAX_RADIO_PACKET_BYTES {
        return None;
    }
    if bytes[6] & !FLAG_RECIPIENT != 0 {
        return None;
    }

    let mut sender = [0u8; 6];
    sender.copy_from_slice(&bytes[8..14]);
    let sender = DeviceId::try_from(sender).ok()?;

    let recipient = if bytes[6] & FLAG_RECIPIENT != 0 {
        let mut recipient = [0u8; 6];
        recipient.copy_from_slice(&bytes[14..20]);
        Some(DeviceId::try_from(recipient).ok()?)
    } else {
        if bytes[14..20] != [0; 6] {
            return None;
        }
        None
    };

    let payload_len = u16::from_le_bytes(bytes[20..22].try_into().ok()?) as usize;
    if payload_len > MAX_PAYLOAD || bytes.len() != APPLICATION_HEADER_BYTES + payload_len {
        return None;
    }

    Some(ApplicationPacket {
        sender,
        recipient,
        payload: &bytes[APPLICATION_HEADER_BYTES..],
    })
}
