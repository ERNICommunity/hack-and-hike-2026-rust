//! Fixed-size Hack and Hike ESP-NOW wire protocol.
//!
//! The radio service never sends Rust struct layouts directly. Packets are
//! encoded explicitly into a small, versioned byte array so two devices built
//! with different firmware revisions can reject incompatible frames safely.

use core::fmt;

pub const PACKET_BYTES: usize = 32;
pub const PROTOCOL_VERSION: u8 = 1;
pub const CAP_IMU: u32 = 1 << 0;
pub const CAP_AUDIO: u32 = 1 << 1;
pub const CAP_DISPLAY: u32 = 1 << 2;
pub const LOCAL_CAPABILITIES: u32 = CAP_IMU | CAP_AUDIO | CAP_DISPLAY;

const MAGIC: [u8; 4] = *b"HNHN";
const KIND_BEACON: u8 = 1;

/// Stable physical-device identity.
///
/// Runtime code constructs this from the ESP32-S3 factory base MAC stored in
/// eFuse. That value is programmed during manufacturing and survives resets and
/// firmware updates without any NVS/flash bookkeeping.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct DeviceId(pub [u8; 6]);

impl DeviceId {
    pub const ZERO: Self = Self([0; 6]);

    pub const fn new(bytes: [u8; 6]) -> Self {
        Self(bytes)
    }

    pub const fn bytes(self) -> [u8; 6] {
        self.0
    }

    pub const fn is_zero(self) -> bool {
        self.0 == [0; 6]
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PacketKind {
    Beacon,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Packet {
    pub kind: PacketKind,
    pub device_id: DeviceId,
    pub sequence: u32,
    pub uptime_ms: u32,
    pub capabilities: u32,
}

impl Packet {
    pub const fn beacon(device_id: DeviceId, sequence: u32, uptime_ms: u32) -> Self {
        Self {
            kind: PacketKind::Beacon,
            device_id,
            sequence,
            uptime_ms,
            capabilities: LOCAL_CAPABILITIES,
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
        out[22..26].copy_from_slice(&self.capabilities.to_le_bytes());
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
        if id == [0; 6] {
            return None;
        }

        Some(Self {
            kind,
            device_id: DeviceId(id),
            sequence: u32::from_le_bytes(bytes[14..18].try_into().ok()?),
            uptime_ms: u32::from_le_bytes(bytes[18..22].try_into().ok()?),
            capabilities: u32::from_le_bytes(bytes[22..26].try_into().ok()?),
        })
    }
}
