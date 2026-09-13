//! ESP-NOW peer discovery and typed messaging.
//!
//! Every device broadcasts a beacon a few times per second; devices that hear
//! each other become peers. Applications send and receive their own
//! `serde`-serializable message types through the [`Network`] handle, either
//! to everyone ([`Network::broadcast`]) or to one peer ([`Network::send_to`]).
//! The wire format, MAC addresses and the radio itself stay private; the
//! protocol and peer table live in `hack_and_hike_core::network`, where they
//! are unit-tested on the host.

mod channels;
mod radio;

use embassy_time::Duration;
use esp_hal::peripherals::WIFI;

pub use channels::Network;
pub use hack_and_hike_core::network::{
    message::{DecodeError, IncomingMessage, SendError},
    protocol::{DeviceId, MAX_PAYLOAD},
    state::{Channel, MAX_PEERS, Peer, Snapshot, Status},
};

pub(crate) use channels::{Endpoints, Runtime, endpoints};
pub(crate) use radio::start;

/// Radio configuration owned by the CPU1 network runtime.
#[derive(Clone, Copy)]
pub(crate) struct Config {
    pub(crate) channel: Channel,
    /// How often this device announces itself.
    pub(crate) beacon_period: Duration,
    /// A peer that stays silent this long is forgotten.
    pub(crate) peer_timeout: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            channel: Channel::new(6),
            beacon_period: Duration::from_millis(250),
            peer_timeout: Duration::from_millis(500),
        }
    }
}

/// CPU1-owned physical resource required by ESP-NOW.
pub(crate) struct Resources {
    pub(crate) wifi: WIFI<'static>,
}
