//! CPU1-owned ESP-NOW discovery service.
//!
//! CPU0 consumes only semantic snapshots. Peer-state logic, radio adaptation,
//! cross-core synchronization, and wire-format details remain private to this
//! capability behind the facade below.

mod channels;
pub(crate) mod protocol;
mod radio;
mod state;

use core::fmt;

use embassy_time::Duration;
use esp_hal::peripherals::WIFI;

pub use channels::Input;
pub use protocol::DeviceId;
pub use radio::start;
pub(crate) use channels::{Endpoints, Runtime, init_endpoints};

pub const MAX_PEERS: usize = 10;
const _: () = assert!(MAX_PEERS > 0);

/// Valid 2.4 GHz ESP-NOW channel number used by this firmware.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Channel(u8);

impl Channel {
    const fn new(number: u8) -> Self {
        assert!(number >= 1 && number <= 14);
        Self(number)
    }

    pub const fn number(self) -> u8 {
        self.0
    }
}

impl fmt::Display for Channel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

pub const DEFAULT_CHANNEL: Channel = Channel::new(6);
pub const DEFAULT_BEACON_PERIOD: Duration = Duration::from_millis(250);
pub const DEFAULT_DEVICE_TIMEOUT: Duration = Duration::from_millis(500);

/// Radio configuration owned by the CPU1 network service.
#[derive(Clone, Copy)]
pub struct Config {
    pub channel: Channel,
    pub beacon_period: Duration,
    pub peer_timeout: Duration,
}

pub const DEFAULT_CONFIG: Config = Config {
    channel: DEFAULT_CHANNEL,
    beacon_period: DEFAULT_BEACON_PERIOD,
    peer_timeout: DEFAULT_DEVICE_TIMEOUT,
};

/// CPU1-owned physical resource required by ESP-NOW.
pub struct Resources {
    pub wifi: WIFI<'static>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Starting,
    Ready,
    PeerPresent,
    Fault,
}

/// Signed received-signal strength in dBm.
///
/// ESP radio metadata exposes the hardware byte representation. Converting it at
/// the service boundary prevents values such as raw `224` from leaking into the
/// application when that byte actually represents `-32 dBm`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RssiDbm(i8);

impl RssiDbm {
    pub(super) fn from_radio_raw(raw: u8) -> Self {
        Self(raw as i8)
    }
}

impl fmt::Display for RssiDbm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// ESP-NOW MAC address kept distinct from the stable physical `DeviceId`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MacAddress([u8; 6]);

impl MacAddress {
    pub(super) fn new(bytes: [u8; 6]) -> Self {
        Self(bytes)
    }
}

impl fmt::Display for MacAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
            self.0[0], self.0[1], self.0[2], self.0[3], self.0[4], self.0[5]
        )
    }
}

/// Presentation-sized data retained for one currently visible peer.
#[derive(Clone, Copy, Debug)]
pub struct PeerSnapshot {
    pub device_id: DeviceId,
    pub rssi_dbm: RssiDbm,
    pub age_ms: u32,
}

/// Replace-latest CPU1→CPU0 network presentation state.
///
/// `peers` is fixed-capacity and uses `Option` for occupancy. `peer_count()` is
/// derived, so count and table contents cannot disagree.
#[derive(Clone, Copy, Debug)]
pub struct Snapshot {
    pub revision: u32,
    pub status: Status,
    pub local_id: DeviceId,
    pub channel: Channel,
    pub peers: [Option<PeerSnapshot>; MAX_PEERS],
    pub tx_packets: u32,
    pub rx_packets: u32,
    pub tx_errors: u32,
    pub rx_invalid: u32,
    pub peer_evictions: u32,
}

impl Snapshot {
    pub fn peer_count(&self) -> usize {
        self.peers.iter().flatten().count()
    }

    pub fn peers(&self) -> impl Iterator<Item = &PeerSnapshot> {
        self.peers.iter().flatten()
    }
}
