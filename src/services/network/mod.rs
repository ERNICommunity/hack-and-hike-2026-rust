//! CPU1-owned ESP-NOW discovery service.
//!
//! CPU0 consumes only semantic snapshots. Peer-state logic, radio adaptation,
//! cross-core synchronization, and wire-format details remain private to this
//! capability behind the facade below.

mod channels;
mod protocol;
mod radio;
mod state;

use core::fmt;

use embassy_time::Duration;
use esp_hal::peripherals::WIFI;

pub(crate) use channels::Input;
pub(crate) use protocol::DeviceId;
pub(crate) use radio::start;
pub(crate) use channels::{Endpoints, Runtime, init_endpoints};

pub(crate) const MAX_PEERS: usize = 10;
const _: () = assert!(MAX_PEERS > 0);

/// Valid 2.4 GHz ESP-NOW channel number used by this firmware.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Channel(u8);

impl Channel {
    const fn new(number: u8) -> Self {
        assert!(number >= 1 && number <= 14);
        Self(number)
    }

    pub(crate) const fn number(self) -> u8 {
        self.0
    }
}

impl fmt::Display for Channel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

pub(crate) const DEFAULT_CHANNEL: Channel = Channel::new(6);
pub(crate) const DEFAULT_BEACON_PERIOD: Duration = Duration::from_millis(250);
pub(crate) const DEFAULT_DEVICE_TIMEOUT: Duration = Duration::from_millis(500);

/// Radio configuration owned by the CPU1 network service.
#[derive(Clone, Copy)]
pub(crate) struct Config {
    pub(crate) channel: Channel,
    pub(crate) beacon_period: Duration,
    pub(crate) peer_timeout: Duration,
}

pub(crate) const DEFAULT_CONFIG: Config = Config {
    channel: DEFAULT_CHANNEL,
    beacon_period: DEFAULT_BEACON_PERIOD,
    peer_timeout: DEFAULT_DEVICE_TIMEOUT,
};

/// CPU1-owned physical resource required by ESP-NOW.
pub(crate) struct Resources {
    pub(crate) wifi: WIFI<'static>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Status {
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
pub(crate) struct RssiDbm(i8);

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
pub(crate) struct MacAddress([u8; 6]);

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
pub(crate) struct PeerSnapshot {
    pub(crate) device_id: DeviceId,
    pub(crate) rssi_dbm: RssiDbm,
    pub(crate) age_ms: u32,
}

/// Replace-latest CPU1→CPU0 network presentation state.
///
/// `peers` is fixed-capacity and uses `Option` for occupancy. `peer_count()` is
/// derived, so count and table contents cannot disagree.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Snapshot {
    pub(crate) revision: u32,
    pub(crate) status: Status,
    pub(crate) local_id: DeviceId,
    pub(crate) channel: Channel,
    pub(crate) peers: [Option<PeerSnapshot>; MAX_PEERS],
    pub(crate) tx_packets: u32,
    pub(crate) rx_packets: u32,
    pub(crate) tx_errors: u32,
    pub(crate) rx_invalid: u32,
    pub(crate) peer_evictions: u32,
}

impl Snapshot {
    pub(crate) fn peer_count(&self) -> usize {
        self.peers.iter().flatten().count()
    }

    pub(crate) fn peers(&self) -> impl Iterator<Item = &PeerSnapshot> {
        self.peers.iter().flatten()
    }
}
