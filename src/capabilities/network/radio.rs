//! CPU1 ESP-NOW driver: beacons, sending, receiving and the peer table.

use core::cell::RefCell;

use embassy_executor::Spawner;
use embassy_futures::select::{Either, select};
use embassy_sync::blocking_mutex::{Mutex, raw::CriticalSectionRawMutex};
use embassy_time::{Instant, Ticker};
use esp_hal::efuse;
use esp_radio::{
    esp_now::{
        BROADCAST_ADDRESS, EspNowManager, EspNowReceiver, EspNowSender, EspNowWifiInterface,
        PeerInfo,
    },
    wifi::WifiController,
};
use log::{error, info};
use static_cell::StaticCell;

use super::{
    Config, Resources,
    channels::Runtime,
    message::{IncomingMessage, OutgoingMessage},
    protocol::{self, DeviceId, MacAddress, RssiDbm},
    state::NetworkState,
};

type SharedState = Mutex<CriticalSectionRawMutex, RefCell<NetworkState>>;

static STATE: StaticCell<SharedState> = StaticCell::new();
static WIFI_CONTROLLER: StaticCell<WifiController<'static>> = StaticCell::new();
static MANAGER: StaticCell<EspNowManager<'static>> = StaticCell::new();

/// Everything the send and receive tasks share.
#[derive(Clone, Copy)]
struct Radio {
    state: &'static SharedState,
    runtime: Runtime,
    local_id: DeviceId,
    config: Config,
}

impl Radio {
    fn with_state<R>(self, f: impl FnOnce(&mut NetworkState) -> R) -> R {
        self.state.lock(|state| f(&mut state.borrow_mut()))
    }

    fn publish(self, now: Instant) {
        let counters = self.runtime.queue_counters();
        let snapshot = self.with_state(|state| state.snapshot(now.as_millis(), counters));
        self.runtime.publish(snapshot);
    }
}

fn physical_device_id() -> DeviceId {
    let mac = efuse::base_mac_address();
    let bytes = <[u8; 6]>::try_from(mac.as_bytes()).expect("a MAC address is six bytes");
    DeviceId::try_from(bytes).expect("the factory MAC address is not all zero")
}

/// Initialize ESP-NOW and spawn the CPU1 send and receive tasks.
///
/// A radio that fails to initialize is reported as [`super::Status::Fault`]
/// instead of panicking, so the rest of the board keeps working.
pub(crate) fn start(spawner: &Spawner, resources: Resources, config: Config, runtime: Runtime) {
    let local_id = physical_device_id();
    let state = STATE.init(Mutex::new(RefCell::new(NetworkState::new(
        local_id,
        config.channel,
        config.peer_timeout.as_millis(),
    ))));
    let radio = Radio {
        state,
        runtime,
        local_id,
        config,
    };
    radio.publish(Instant::now());

    let controller = match WifiController::new(resources.wifi, Default::default()) {
        Ok(controller) => WIFI_CONTROLLER.init(controller),
        Err(err) => {
            error!("ESP-NOW radio init failed: {:?}", err);
            radio.with_state(NetworkState::mark_fault);
            radio.publish(Instant::now());
            return;
        }
    };

    let esp_now = controller.esp_now();
    if let Err(err) = esp_now.set_channel(config.channel.number()) {
        error!("ESP-NOW channel {} failed: {:?}", config.channel, err);
        radio.with_state(NetworkState::mark_fault);
        radio.publish(Instant::now());
        return;
    }
    let version = esp_now.version().unwrap_or(0);
    let (manager, sender, receiver) = esp_now.split();
    let manager = MANAGER.init(manager);

    radio.with_state(NetworkState::mark_ready);
    radio.publish(Instant::now());

    spawner.spawn(
        receive_task(manager, receiver, radio).expect("ESP-NOW receive task already spawned"),
    );
    spawner.spawn(
        transmit_task(manager, sender, radio).expect("ESP-NOW transmit task already spawned"),
    );

    info!(
        "ESP-NOW started: id={} channel={} version={}",
        local_id, config.channel, version
    );
}

/// Send queued application messages as they arrive and a beacon on every tick.
#[embassy_executor::task]
async fn transmit_task(
    manager: &'static EspNowManager<'static>,
    mut sender: EspNowSender<'static>,
    radio: Radio,
) {
    let mut beacons = Ticker::every(radio.config.beacon_period);
    let mut frame = [0u8; protocol::MAX_RADIO_PACKET_BYTES];

    loop {
        let now = Instant::now();
        match select(radio.runtime.next_outgoing(), beacons.next()).await {
            Either::First(message) => send_message(&mut sender, radio, &message, &mut frame).await,
            Either::Second(()) => {
                let beacon = radio.with_state(|state| state.next_beacon(now.as_millis()));
                let result = sender
                    .send_async(&BROADCAST_ADDRESS, &beacon.encode())
                    .await;
                radio.with_state(|state| record_send(state, result.is_ok()));

                let expired = radio.with_state(|state| state.expire_peers(now.as_millis()));
                for mac in expired {
                    forget_radio_peer(manager, mac);
                }
            }
        }
        radio.publish(Instant::now());
    }
}

async fn send_message(
    sender: &mut EspNowSender<'static>,
    radio: Radio,
    message: &OutgoingMessage,
    frame: &mut [u8; protocol::MAX_RADIO_PACKET_BYTES],
) {
    let destination = match message.recipient {
        None => Some(BROADCAST_ADDRESS),
        Some(peer) => radio
            .with_state(|state| state.route_for(peer))
            .map(|mac| mac.0),
    };
    let encoded_len =
        protocol::encode_application(radio.local_id, message.recipient, &message.payload, frame);

    let sent = match (destination, encoded_len) {
        (Some(destination), Some(len)) => {
            sender.send_async(&destination, &frame[..len]).await.is_ok()
        }
        _ => false,
    };
    radio.with_state(|state| record_send(state, sent));
}

fn record_send(state: &mut NetworkState, ok: bool) {
    if ok {
        state.record_send_ok();
    } else {
        state.record_send_error();
    }
}

fn forget_radio_peer(manager: &EspNowManager<'static>, mac: MacAddress) {
    if manager.peer_exists(&mac.0)
        && let Err(err) = manager.remove_peer(&mac.0)
    {
        error!("ESP-NOW could not remove peer {}: {:?}", mac, err);
    }
}

/// Decode received frames, maintain the peer table and hand application
/// messages to CPU0.
#[embassy_executor::task]
async fn receive_task(
    manager: &'static EspNowManager<'static>,
    mut receiver: EspNowReceiver<'static>,
    radio: Radio,
) {
    loop {
        let received = receiver.receive_async().await;
        let now = Instant::now();
        let broadcast = received.info.dst_address == BROADCAST_ADDRESS;

        let (sender_id, application) = match protocol::decode_frame(received.data()) {
            Some(protocol::DecodedFrame::Beacon(beacon)) => (Some(beacon.device_id), None),
            Some(protocol::DecodedFrame::Application(packet)) => {
                let addressed_correctly = match packet.recipient {
                    None => broadcast,
                    Some(recipient) => recipient == radio.local_id && !broadcast,
                };
                (addressed_correctly.then_some(packet.sender), Some(packet))
            }
            None => (None, None),
        };
        let Some(sender_id) = sender_id else {
            radio.with_state(NetworkState::record_invalid_receive);
            radio.publish(now);
            continue;
        };
        if sender_id == radio.local_id {
            continue;
        }

        let mac = MacAddress(received.info.src_address);
        let rssi = RssiDbm::from_dbm(received.info.rx_control.rssi);
        let outcome =
            radio.with_state(|state| state.record_receive(sender_id, mac, rssi, now.as_millis()));
        if let Some(evicted) = outcome.evicted {
            forget_radio_peer(manager, evicted);
        }
        if outcome.is_new {
            info!("ESP-NOW peer found: id={} rssi={} dBm", sender_id, rssi.0);
        }
        if broadcast && !manager.peer_exists(&mac.0) {
            let added = manager.add_peer(PeerInfo {
                interface: EspNowWifiInterface::Station,
                peer_address: mac.0,
                lmk: None,
                channel: Some(radio.config.channel.number()),
                encrypt: false,
            });
            if let Err(err) = added {
                error!("ESP-NOW could not add peer {}: {:?}", mac, err);
            }
        }

        if let Some(packet) = application
            && let Some(message) = IncomingMessage::from_bytes(packet.sender, packet.payload)
        {
            radio.runtime.deliver(message);
        }
        radio.publish(now);
    }
}
