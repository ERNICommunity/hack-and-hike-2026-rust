//! CPU1-owned ESP-NOW service.
//!
//! The Wi-Fi radio and ESP-NOW handles never leave CPU1. The service broadcasts
//! a small fixed-size discovery beacon, tracks a bounded peer table, and
//! publishes only a replace-latest [`Snapshot`] for CPU0 presentation.

use core::cell::RefCell;

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

pub const MAX_PEERS: usize = 4;
pub const DEFAULT_CHANNEL: u8 = 6;

#[derive(Clone, Copy)]
pub struct Config {
    pub channel: u8,
    pub beacon_period: Duration,
    pub peer_timeout: Duration,
}

pub const DEFAULT_CONFIG: Config = Config {
    channel: DEFAULT_CHANNEL,
    beacon_period: Duration::from_secs(1),
    peer_timeout: Duration::from_secs(5),
};

/// CPU1-owned physical resource required by ESP-NOW.
pub struct Resources {
    pub wifi: WIFI<'static>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum Status {
    Starting = 0,
    Ready = 1,
    PeerPresent = 2,
    Fault = 3,
}

#[derive(Clone, Copy, Debug)]
pub struct PeerSnapshot {
    pub present: bool,
    pub device_id: protocol::DeviceId,
    pub mac: [u8; 6],
    pub rssi_dbm: i16,
    pub age_ms: u32,
    pub rx_packets: u32,
    pub remote_uptime_ms: u32,
    pub capabilities: u32,
}

impl PeerSnapshot {
    pub const EMPTY: Self = Self {
        present: false,
        device_id: protocol::DeviceId::ZERO,
        mac: [0; 6],
        rssi_dbm: 0,
        age_ms: 0,
        rx_packets: 0,
        remote_uptime_ms: 0,
        capabilities: 0,
    };
}

#[derive(Clone, Copy, Debug)]
pub struct Snapshot {
    pub revision: u32,
    pub status: Status,
    pub local_id: protocol::DeviceId,
    pub channel: u8,
    pub peer_count: u8,
    pub peers: [PeerSnapshot; MAX_PEERS],
    pub tx_packets: u32,
    pub rx_packets: u32,
    pub tx_errors: u32,
    pub rx_invalid: u32,
    pub peer_evictions: u32,
}

#[derive(Clone, Copy)]
struct PeerState {
    present: bool,
    device_id: protocol::DeviceId,
    mac: [u8; 6],
    rssi_dbm: i16,
    last_seen_ms: u64,
    rx_packets: u32,
    remote_uptime_ms: u32,
    capabilities: u32,
}

impl PeerState {
    const EMPTY: Self = Self {
        present: false,
        device_id: protocol::DeviceId::ZERO,
        mac: [0; 6],
        rssi_dbm: 0,
        last_seen_ms: 0,
        rx_packets: 0,
        remote_uptime_ms: 0,
        capabilities: 0,
    };
}

struct NetworkState {
    revision: u32,
    status: Status,
    local_id: protocol::DeviceId,
    channel: u8,
    peer_timeout_ms: u64,
    next_sequence: u32,
    peers: [PeerState; MAX_PEERS],
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
            peers: [PeerState::EMPTY; MAX_PEERS],
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
        mac: [u8; 6],
        rssi_dbm: i16,
        now: Instant,
    ) -> bool {
        self.rx_packets = self.rx_packets.wrapping_add(1);
        let now_ms = now.as_millis();

        if let Some(index) = self
            .peers
            .iter()
            .position(|peer| peer.present && peer.device_id == packet.device_id)
        {
            let peer = &mut self.peers[index];
            peer.mac = mac;
            peer.rssi_dbm = rssi_dbm;
            peer.last_seen_ms = now_ms;
            peer.rx_packets = peer.rx_packets.wrapping_add(1);
            peer.remote_uptime_ms = packet.uptime_ms;
            peer.capabilities = packet.capabilities;
            self.bump_revision();
            return false;
        }

        let index = self
            .peers
            .iter()
            .position(|peer| !peer.present)
            .unwrap_or_else(|| {
                let mut oldest_index = 0usize;
                let mut oldest_seen = self.peers[0].last_seen_ms;
                for (index, peer) in self.peers.iter().enumerate().skip(1) {
                    if peer.last_seen_ms < oldest_seen {
                        oldest_seen = peer.last_seen_ms;
                        oldest_index = index;
                    }
                }
                self.peer_evictions = self.peer_evictions.wrapping_add(1);
                diagnostics::record_network_peer_eviction();
                oldest_index
            });

        self.peers[index] = PeerState {
            present: true,
            device_id: packet.device_id,
            mac,
            rssi_dbm,
            last_seen_ms: now_ms,
            rx_packets: 1,
            remote_uptime_ms: packet.uptime_ms,
            capabilities: packet.capabilities,
        };
        self.bump_revision();
        true
    }

    fn expire_peers(&mut self, now: Instant) {
        let now_ms = now.as_millis();
        let mut changed = false;
        for peer in &mut self.peers {
            if peer.present && now_ms.saturating_sub(peer.last_seen_ms) > self.peer_timeout_ms {
                *peer = PeerState::EMPTY;
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
        let mut peers = [PeerSnapshot::EMPTY; MAX_PEERS];
        let mut peer_count = 0u8;

        for (source, target) in self.peers.iter().zip(peers.iter_mut()) {
            if !source.present {
                continue;
            }
            peer_count = peer_count.saturating_add(1);
            *target = PeerSnapshot {
                present: true,
                device_id: source.device_id,
                mac: source.mac,
                rssi_dbm: source.rssi_dbm,
                age_ms: now_ms.saturating_sub(source.last_seen_ms).min(u32::MAX as u64) as u32,
                rx_packets: source.rx_packets,
                remote_uptime_ms: source.remote_uptime_ms,
                capabilities: source.capabilities,
            };
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
            peer_count,
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

/// Take the newest network state, if CPU1 published one since the previous take.
pub fn take_latest() -> Option<Snapshot> {
    LATEST.try_take()
}

fn physical_device_id() -> protocol::DeviceId {
    let mac = efuse::base_mac_address();
    let mut bytes = [0u8; 6];
    bytes.copy_from_slice(mac.as_bytes());
    protocol::DeviceId::new(bytes)
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
    if let Err(error) = esp_now.set_channel(config.channel) {
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
        receive_task(manager, receiver, config)
            .expect("Failed to allocate CPU1 ESP-NOW receive task"),
    );
    spawner.spawn(
        beacon_task(sender, config).expect("Failed to allocate CPU1 ESP-NOW beacon task"),
    );

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
) {
    loop {
        let received = receiver.receive_async().await;
        let Some(packet) = protocol::Packet::decode(received.data()) else {
            diagnostics::record_network_rx_invalid();
            let _ = with_state(NetworkState::record_invalid_receive);
            publish_snapshot(Instant::now());
            continue;
        };

        let local_id = with_state(|state| state.local_id).unwrap_or(protocol::DeviceId::ZERO);
        if packet.device_id == local_id {
            continue;
        }

        diagnostics::record_network_rx_packet();
        let now = Instant::now();
        let is_new = with_state(|state| {
            state.record_receive(
                packet,
                received.info.src_address,
                received.info.rx_control.rssi.clamp(i16::MIN as i32, i16::MAX as i32) as i16,
                now,
            )
        })
        .unwrap_or(false);

        if received.info.dst_address == BROADCAST_ADDRESS
            && !manager.peer_exists(&received.info.src_address)
        {
            let _ = manager.add_peer(PeerInfo {
                interface: EspNowWifiInterface::Station,
                peer_address: received.info.src_address,
                lmk: None,
                channel: Some(config.channel),
                encrypt: false,
            });
        }

        if is_new {
            ::log::info!(
                "ESP-NOW peer: id={} mac={:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X} rssi={} dBm",
                packet.device_id,
                received.info.src_address[0],
                received.info.src_address[1],
                received.info.src_address[2],
                received.info.src_address[3],
                received.info.src_address[4],
                received.info.src_address[5],
                received.info.rx_control.rssi,
            );
        }

        publish_snapshot(now);
    }
}
