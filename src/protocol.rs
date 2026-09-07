//! Fixed-size Hack and Hike ESP-NOW wire protocol.
//!
//! The radio service never sends Rust struct layouts directly. Packets are
//! encoded explicitly into a small, versioned byte array so two devices built
//! with different firmware revisions can reject incompatible frames safely.

use core::fmt;

pub const PACKET_BYTES: usize = 32;
pub const PROTOCOL_VERSION: u8 = 1;

const MAGIC: [u8; 4] = *b"HNHN";
const KIND_BEACON: u8 = 1;

/// Stable physical-device identity derived from the factory eFuse MAC.
///
/// An all-zero identifier is invalid and cannot be represented as `DeviceId`.
/// The inner bytes remain private so wire-format code is the only place that can
/// depend on their layout.
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
        write!(
            f,
            "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
            self.0[0], self.0[1], self.0[2], self.0[3], self.0[4], self.0[5]
        )
    }
}

/// Capability bits carried by discovery beacons.
///
/// A newtype keeps protocol flags distinct from unrelated counters while
/// preserving the exact four-byte wire representation. Unknown bits are kept so
/// newer peers remain forward-compatible with older firmware.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Capabilities(u32);

impl Capabilities {
    pub const IMU: Self = Self(1 << 0);
    pub const AUDIO: Self = Self(1 << 1);
    pub const DISPLAY: Self = Self(1 << 2);
    pub const LOCAL: Self = Self(Self::IMU.0 | Self::AUDIO.0 | Self::DISPLAY.0);

    pub const fn bits(self) -> u32 {
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
pub enum PacketKind {
    Beacon,
}

/// Decoded semantic packet. Serialization is always explicit via `encode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Packet {
    pub kind: PacketKind,
    pub device_id: DeviceId,
    pub sequence: u32,
    pub uptime_ms: u32,
    pub capabilities: Capabilities,
}

impl Packet {
    pub const fn beacon(device_id: DeviceId, sequence: u32, uptime_ms: u32) -> Self {
        Self {
            kind: PacketKind::Beacon,
            device_id,
            sequence,
            uptime_ms,
            capabilities: Capabilities::LOCAL,
        }
    }

    pub fn encode(self) -> [u8; PACKET_BYTES] {
        let mut out = [0u8; PACKET_BYTES];
        out[0..4].copy_from_slice(&MAGIC);
        out[4] = PROTOCOL_VERSION;
        out[5] = match self.kind {
            PacketKind::Beacon => KIND_BEACON,
        };
        // byte 6 is flags, byte 7 is reserved for future protocol use.
        out[8..14].copy_from_slice(&self.device_id.0);
        out[14..18].copy_from_slice(&self.sequence.to_le_bytes());
        out[18..22].copy_from_slice(&self.uptime_ms.to_le_bytes());
        out[22..26].copy_from_slice(&self.capabilities.bits().to_le_bytes());
        out
    }

    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != PACKET_BYTES || bytes[0..4] != MAGIC {
            return None;
        }
        if bytes[4] != PROTOCOL_VERSION {
            return None;
        }

        let kind = match bytes[5] {
            KIND_BEACON => PacketKind::Beacon,
            _ => return None,
        };

        let mut id = [0u8; 6];
        id.copy_from_slice(&bytes[8..14]);

        Some(Self {
            kind,
            device_id: DeviceId::try_from(id).ok()?,
            sequence: u32::from_le_bytes(bytes[14..18].try_into().ok()?),
            uptime_ms: u32::from_le_bytes(bytes[18..22].try_into().ok()?),
            capabilities: Capabilities::from_bits(u32::from_le_bytes(
                bytes[22..26].try_into().ok()?,
            )),
        })
    }
}
