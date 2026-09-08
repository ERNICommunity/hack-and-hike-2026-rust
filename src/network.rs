//! CPU1-owned ESP-NOW service.
//!
//! The Wi-Fi radio and ESP-NOW handles never leave CPU1. The service broadcasts
//! a small fixed-size discovery beacon, tracks a bounded peer table, and
//! publishes a replace-latest `Snapshot` for the CPU0 network reader.

use core::{cell::RefCell, fmt};

use critical_section::Mutex;
use embassy_executor::Spawner;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use embassy_time::{Duration, Instant, Ticker};
use esp_hal::{efuse, peripherals::WIFI};
use esp_radio::{
    esp_now::{
        BROADCAST_ADDRESS, EspNowManager, EspNowReceiver, EspNowSender, EspNowWifiInterface,
        PeerInfo,
    },
    wifi::{self, WifiController},
};
use static_cell::StaticCell;

use crate::{diagnostics, protocol};

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
    peer_timeout: Duration::from_secs(1),
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
    fn from_radio_raw(raw: u8) -> Self {
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
    fn new(bytes: [u8; 6]) -> Self {
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
    pub device_id: protocol::DeviceId,
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
    pub local_id: protocol::DeviceId,
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

#[derive(Clone, Copy)]
struct PeerState {
    device_id: protocol::DeviceId,
    rssi_dbm: RssiDbm,
    last_seen_ms: u64,
    rx_packets: u32,
}

struct NetworkState {
    revision: u32,
    status: Status,
    local_id: protocol::DeviceId,
    channel: Channel,
    peer_timeout_ms: u64,
    next_sequence: u32,
    peers: [Option<PeerState>; MAX_PEERS],
    tx_packets: u32,
    rx_packets: u32,
    tx_errors: u32,
    rx_invalid: u32,
    peer_evictions: u32,
}

impl NetworkState {
    fn new(local_id: protocol::DeviceId, config: Config) -> Self {
        Self {
            revision: 0,
            status: Status::Starting,
            local_id,
            channel: config.channel,
            peer_timeout_ms: config.peer_timeout.as_millis(),
            next_sequence: 1,
            peers: [None; MAX_PEERS],
            tx_packets: 0,
            rx_packets: 0,
            tx_errors: 0,
            rx_invalid: 0,
            peer_evictions: 0,
        }
    }

    fn mark_ready(&mut self) {
        self.status = Status::Ready;
        self.bump_revision();
    }

    fn mark_fault(&mut self) {
        self.status = Status::Fault;
        self.bump_revision();
    }

    fn bump_revision(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    fn next_beacon(&mut self, now: Instant) -> protocol::Packet {
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.wrapping_add(1);
        protocol::Packet::beacon(self.local_id, sequence, now.as_millis() as u32)
    }

    fn record_send_ok(&mut self) {
        self.tx_packets = self.tx_packets.wrapping_add(1);
        self.bump_revision();
    }

    fn record_send_error(&mut self) {
        self.tx_errors = self.tx_errors.wrapping_add(1);
        self.bump_revision();
    }

    fn record_invalid_receive(&mut self) {
        self.rx_invalid = self.rx_invalid.wrapping_add(1);
        self.bump_revision();
    }

    fn record_receive(
        &mut self,
        packet: protocol::Packet,
        rssi_dbm: RssiDbm,
        now: Instant,
    ) -> bool {
        self.rx_packets = self.rx_packets.wrapping_add(1);
        let now_ms = now.as_millis();

        if let Some(peer) = self
            .peers
            .iter_mut()
            .flatten()
            .find(|peer| peer.device_id == packet.device_id)
        {
            peer.rssi_dbm = rssi_dbm;
            peer.last_seen_ms = now_ms;
            peer.rx_packets = peer.rx_packets.wrapping_add(1);
            self.bump_revision();
            return false;
        }

        let index = self.slot_for_new_peer();
        self.peers[index] = Some(PeerState {
            device_id: packet.device_id,
            rssi_dbm,
            last_seen_ms: now_ms,
            rx_packets: 1,
        });
        self.bump_revision();
        true
    }

    fn slot_for_new_peer(&mut self) -> usize {
        let mut oldest_index = 0usize;
        let mut oldest_seen = u64::MAX;

        for (index, peer) in self.peers.iter().enumerate() {
            let Some(peer) = peer else {
                return index;
            };
            if peer.last_seen_ms < oldest_seen {
                oldest_seen = peer.last_seen_ms;
                oldest_index = index;
            }
        }

        self.peer_evictions = self.peer_evictions.wrapping_add(1);
        diagnostics::record_network_peer_eviction();
        oldest_index
    }

    fn expire_peers(&mut self, now: Instant) {
        let now_ms = now.as_millis();
        let mut changed = false;
        for peer in &mut self.peers {
            if peer
                .as_ref()
                .is_some_and(|peer| now_ms.saturating_sub(peer.last_seen_ms) > self.peer_timeout_ms)
            {
                *peer = None;
                changed = true;
            }
        }
        if changed {
            self.bump_revision();
        }
    }

    fn snapshot(&mut self, now: Instant) -> Snapshot {
        self.expire_peers(now);
        let now_ms = now.as_millis();
        let peer_count = self.peers.iter().flatten().count();
        let mut peers = [None; MAX_PEERS];

        for (target, source) in peers.iter_mut().zip(self.peers.iter().flatten()) {
            *target = Some(PeerSnapshot {
                device_id: source.device_id,
                rssi_dbm: source.rssi_dbm,
                age_ms: now_ms
                    .saturating_sub(source.last_seen_ms)
                    .min(u32::MAX as u64) as u32,
            });
        }

        if self.status != Status::Fault && self.status != Status::Starting {
            self.status = if peer_count == 0 {
                Status::Ready
            } else {
                Status::PeerPresent
            };
        }

        Snapshot {
            revision: self.revision,
            status: self.status,
            local_id: self.local_id,
            channel: self.channel,
            peers,
            tx_packets: self.tx_packets,
            rx_packets: self.rx_packets,
            tx_errors: self.tx_errors,
            rx_invalid: self.rx_invalid,
            peer_evictions: self.peer_evictions,
        }
    }
}

static STATE: Mutex<RefCell<Option<NetworkState>>> = Mutex::new(RefCell::new(None));
static LATEST: Signal<CriticalSectionRawMutex, Snapshot> = Signal::new();
static WIFI_CONTROLLER: StaticCell<WifiController<'static>> = StaticCell::new();

fn with_state<R>(f: impl FnOnce(&mut NetworkState) -> R) -> Option<R> {
    critical_section::with(|cs| {
        let mut state = STATE.borrow(cs).borrow_mut();
        state.as_mut().map(f)
    })
}

fn publish_snapshot(now: Instant) {
    if let Some(snapshot) = with_state(|state| state.snapshot(now)) {
        LATEST.signal(snapshot);
    }
}

/// Low-level replace-latest take used by the CPU0 network reader handle.
pub fn take_latest() -> Option<Snapshot> {
    LATEST.try_take()
}

fn physical_device_id() -> protocol::DeviceId {
    let mac = efuse::base_mac_address();
    let mut bytes = [0u8; 6];
    bytes.copy_from_slice(mac.as_bytes());
    protocol::DeviceId::try_from(bytes).expect("factory eFuse MAC must not be all zero")
}

/// Initialize ESP-NOW and spawn its CPU1-owned send/receive tasks.
///
/// Initialization failure is reported as a network fault instead of panicking
/// the rest of the firmware, so audio/IMU/UI can keep running for diagnostics.
pub fn start(spawner: &Spawner, resources: Resources, config: Config) {
    let local_id = physical_device_id();
    critical_section::with(|cs| {
        *STATE.borrow(cs).borrow_mut() = Some(NetworkState::new(local_id, config));
    });
    publish_snapshot(Instant::now());

    let (controller, interfaces) = match wifi::new(resources.wifi, Default::default()) {
        Ok(result) => result,
        Err(error) => {
            diagnostics::record_network_init_error();
            let _ = with_state(NetworkState::mark_fault);
            publish_snapshot(Instant::now());
            ::log::error!("ESP-NOW radio init failed: {:?}", error);
            return;
        }
    };
    let _controller = WIFI_CONTROLLER.init(controller);

    let esp_now = interfaces.esp_now;
    if let Err(error) = esp_now.set_channel(config.channel.number()) {
        diagnostics::record_network_init_error();
        let _ = with_state(NetworkState::mark_fault);
        publish_snapshot(Instant::now());
        ::log::error!("ESP-NOW channel {} failed: {:?}", config.channel, error);
        return;
    }

    let version = esp_now.version().unwrap_or(0);
    let (manager, sender, receiver) = esp_now.split();
    let _ = with_state(NetworkState::mark_ready);
    publish_snapshot(Instant::now());

    spawner.spawn(
        receive_task(manager, receiver, config, local_id)
            .expect("Failed to allocate CPU1 ESP-NOW receive task"),
    );
    spawner
        .spawn(beacon_task(sender, config).expect("Failed to allocate CPU1 ESP-NOW beacon task"));

    ::log::info!(
        "ESP-NOW started: id={} channel={} version={}",
        local_id,
        config.channel,
        version
    );
}

#[embassy_executor::task]
async fn beacon_task(mut sender: EspNowSender<'static>, config: Config) {
    let mut ticker = Ticker::every(config.beacon_period);

    loop {
        let now = Instant::now();
        let Some(packet) = with_state(|state| state.next_beacon(now)) else {
            ticker.next().await;
            continue;
        };
        let payload = packet.encode();

        match sender.send_async(&BROADCAST_ADDRESS, &payload).await {
            Ok(()) => {
                diagnostics::record_network_tx_packet();
                let _ = with_state(NetworkState::record_send_ok);
            }
            Err(_) => {
                diagnostics::record_network_tx_error();
                let _ = with_state(NetworkState::record_send_error);
            }
        }

        publish_snapshot(Instant::now());
        ticker.next().await;
    }
}

#[embassy_executor::task]
async fn receive_task(
    manager: EspNowManager<'static>,
    mut receiver: EspNowReceiver<'static>,
    config: Config,
    local_id: protocol::DeviceId,
) {
    loop {
        let received = receiver.receive_async().await;
        let Some(packet) = protocol::Packet::decode(received.data()) else {
            diagnostics::record_network_rx_invalid();
            let _ = with_state(NetworkState::record_invalid_receive);
            publish_snapshot(Instant::now());
            continue;
        };

        if packet.device_id == local_id {
            continue;
        }

        diagnostics::record_network_rx_packet();
        let now = Instant::now();
        let mac = MacAddress::new(received.info.src_address);
        let rssi = RssiDbm::from_radio_raw(received.info.rx_control.rssi as u8);
        let is_new = with_state(|state| state.record_receive(packet, rssi, now)).unwrap_or(false);

        if received.info.dst_address == BROADCAST_ADDRESS
            && !manager.peer_exists(&received.info.src_address)
        {
            let _ = manager.add_peer(PeerInfo {
                interface: EspNowWifiInterface::Station,
                peer_address: received.info.src_address,
                lmk: None,
                channel: Some(config.channel.number()),
                encrypt: false,
            });
        }

        if is_new {
            ::log::info!(
                "ESP-NOW peer: id={} mac={} rssi={} dBm uptime={} ms cap=0x{:08X}",
                packet.device_id,
                mac,
                rssi,
                packet.uptime_ms,
                packet.capabilities,
            );
        }

        publish_snapshot(now);
    }
}
