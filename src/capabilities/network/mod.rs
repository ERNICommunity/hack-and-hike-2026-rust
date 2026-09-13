//! CPU1-owned ESP-NOW discovery and typed messaging capability.
//!
//! Applications see stable `DeviceId`s, peer health, typed postcard payloads and
//! bounded send/receive queues. ESP-NOW MAC addresses, explicit wire envelopes,
//! radio adaptation and cross-core synchronization remain private.

mod channels;
mod message;
mod protocol;
mod radio;
mod state;

use core::fmt;

use embassy_time::Duration;
use esp_hal::peripherals::WIFI;

pub use channels::Network;
pub(crate) use channels::{Endpoints, Runtime, init_endpoints};
pub use message::{IncomingMessage, SendError};
pub use protocol::{DeviceId, MAX_PAYLOAD};
pub(crate) use radio::start;

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

pub(crate) const DEFAULT_CHANNEL: Channel = Channel::new(6);
pub(crate) const DEFAULT_BEACON_PERIOD: Duration = Duration::from_millis(250);
pub(crate) const DEFAULT_DEVICE_TIMEOUT: Duration = Duration::from_millis(500);

/// Radio configuration owned by the CPU1 network capability.
#[derive(Clone, Copy)]
pub(crate) struct Config {
    pub channel: Channel,
    pub beacon_period: Duration,
    pub peer_timeout: Duration,
}

pub(crate) const DEFAULT_CONFIG: Config = Config {
    channel: DEFAULT_CHANNEL,
    beacon_period: DEFAULT_BEACON_PERIOD,
    peer_timeout: DEFAULT_DEVICE_TIMEOUT,
};

/// CPU1-owned physical resource required by ESP-NOW.
pub(crate) struct Resources {
    pub wifi: WIFI<'static>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Starting,
    Ready,
    PeerPresent,
    Fault,
}

/// Signed received-signal strength in dBm kept private to radio/state code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RssiDbm(i8);

impl RssiDbm {
    fn from_radio_raw(raw: u8) -> Self {
        Self(raw as i8)
    }

    const fn get(self) -> i8 {
        self.0
    }
}

impl fmt::Display for RssiDbm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// ESP-NOW MAC address kept distinct from the stable physical `DeviceId` and
/// never exposed through the application-facing capability API.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct MacAddress([u8; 6]);

impl MacAddress {
    fn new(bytes: [u8; 6]) -> Self {
        Self(bytes)
    }

    const fn bytes(self) -> [u8; 6] {
        self.0
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

/// One currently discovered peer. Routing MAC addresses stay private.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Peer {
    pub id: DeviceId,
    pub rssi_dbm: i8,
    pub age_ms: u32,
    pub expires_in_ms: u32,
}

/// Replace-latest diagnostics/peer snapshot cached by the CPU0 `Network` handle.
#[derive(Clone, Copy, Debug)]
pub struct Snapshot {
    pub revision: u32,
    pub status: Status,
    pub local_id: DeviceId,
    pub channel: Channel,
    pub peers: [Option<Peer>; MAX_PEERS],
    pub tx_packets: u32,
    pub rx_packets: u32,
    pub tx_errors: u32,
    pub rx_invalid: u32,
    pub peer_evictions: u32,
    pub tx_queue_full: u32,
    pub rx_queue_full: u32,
}

impl Snapshot {
    pub fn peer_count(&self) -> usize {
        self.peers.iter().flatten().count()
    }

    pub fn peers(&self) -> impl Iterator<Item = &Peer> {
        self.peers.iter().flatten()
    }
}
