//! CPU1 ESP-NOW adapter translating radio events into network capability operations.

use core::cell::RefCell;

use critical_section::Mutex;
use embassy_executor::Spawner;
use embassy_time::{Duration, Instant, Ticker};
use esp_hal::efuse;
use esp_radio::{
    esp_now::{
        BROADCAST_ADDRESS, EspNowManager, EspNowReceiver, EspNowSender, EspNowWifiInterface,
        PeerInfo,
    },
    wifi::{self, WifiController},
};
use static_cell::StaticCell;

use crate::support::diagnostics;

use super::{
    Config, MacAddress, Resources, RssiDbm, channels::Runtime, protocol, state::NetworkState,
};

const TX_POLL_PERIOD: Duration = Duration::from_millis(10);

static STATE: Mutex<RefCell<Option<NetworkState>>> = Mutex::new(RefCell::new(None));
static WIFI_CONTROLLER: StaticCell<WifiController<'static>> = StaticCell::new();

fn with_state<R>(f: impl FnOnce(&mut NetworkState) -> R) -> Option<R> {
    critical_section::with(|cs| {
        let mut state = STATE.borrow(cs).borrow_mut();
        state.as_mut().map(f)
    })
}

fn publish_snapshot(runtime: Runtime, now: Instant) {
    if let Some(snapshot) = with_state(|state| state.snapshot(now.as_millis())) {
        runtime.publish(snapshot);
    }
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
pub(crate) fn start(spawner: &Spawner, resources: Resources, config: Config, runtime: Runtime) {
    let local_id = physical_device_id();
    critical_section::with(|cs| {
        *STATE.borrow(cs).borrow_mut() = Some(NetworkState::new(
            local_id,
            config.channel,
            config.peer_timeout.as_millis(),
        ));
    });
    publish_snapshot(runtime, Instant::now());

    let (controller, interfaces) = match wifi::new(resources.wifi, Default::default()) {
        Ok(result) => result,
        Err(error) => {
            diagnostics::record_network_init_error();
            let _ = with_state(NetworkState::mark_fault);
            publish_snapshot(runtime, Instant::now());
            ::log::error!("ESP-NOW radio init failed: {:?}", error);
            return;
        }
    };
    let _controller = WIFI_CONTROLLER.init(controller);

    let esp_now = interfaces.esp_now;
    if let Err(error) = esp_now.set_channel(config.channel.number()) {
        diagnostics::record_network_init_error();
        let _ = with_state(NetworkState::mark_fault);
        publish_snapshot(runtime, Instant::now());
        ::log::error!("ESP-NOW channel {} failed: {:?}", config.channel, error);
        return;
    }

    let version = esp_now.version().unwrap_or(0);
    let (manager, sender, receiver) = esp_now.split();
    let _ = with_state(NetworkState::mark_ready);
    publish_snapshot(runtime, Instant::now());

    spawner.spawn(
        receive_task(manager, receiver, config, local_id, runtime)
            .expect("Failed to allocate CPU1 ESP-NOW receive task"),
    );
    spawner.spawn(
        transmit_task(sender, config, local_id, runtime)
            .expect("Failed to allocate CPU1 ESP-NOW transmit task"),
    );

    ::log::info!(
        "ESP-NOW started: id={} channel={} version={}",
        local_id,
        config.channel,
        version
    );
}

#[embassy_executor::task]
async fn transmit_task(
    mut sender: EspNowSender<'static>,
    config: Config,
    local_id: protocol::DeviceId,
    runtime: Runtime,
) {
    let mut ticker = Ticker::every(TX_POLL_PERIOD);
    let mut next_beacon_ms = 0u64;
    let mut encoded = [0u8; protocol::MAX_RADIO_PACKET_BYTES];

    loop {
        while let Some(message) = runtime.try_take_outgoing() {
            let destination = match message.recipient {
                None => Some(BROADCAST_ADDRESS),
                Some(device_id) => with_state(|state| state.route_for(device_id))
                    .flatten()
                    .map(MacAddress::bytes),
            };

            let Some(destination) = destination else {
                diagnostics::record_network_tx_error();
                let _ = with_state(NetworkState::record_send_error);
                continue;
            };
            let Some(encoded_len) = protocol::encode_application(
                local_id,
                message.recipient,
                &message.payload[..message.len],
                &mut encoded,
            ) else {
                diagnostics::record_network_tx_error();
                let _ = with_state(NetworkState::record_send_error);
                continue;
            };

            match sender.send_async(&destination, &encoded[..encoded_len]).await {
                Ok(()) => {
                    diagnostics::record_network_tx_packet();
                    let _ = with_state(NetworkState::record_send_ok);
                }
                Err(_) => {
                    diagnostics::record_network_tx_error();
                    let _ = with_state(NetworkState::record_send_error);
                }
            }
        }

        let now = Instant::now();
        if now.as_millis() >= next_beacon_ms {
            if let Some(packet) = with_state(|state| state.next_beacon(now.as_millis())) {
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
            }
            next_beacon_ms = now
                .as_millis()
                .saturating_add(config.beacon_period.as_millis());
        }

        publish_snapshot(runtime, Instant::now());
        ticker.next().await;
    }
}

#[embassy_executor::task]
async fn receive_task(
    manager: EspNowManager<'static>,
    mut receiver: EspNowReceiver<'static>,
    config: Config,
    local_id: protocol::DeviceId,
    runtime: Runtime,
) {
    loop {
        let received = receiver.receive_async().await;
        let Some(frame) = protocol::decode_frame(received.data()) else {
            record_invalid(runtime);
            continue;
        };

        let mac = MacAddress::new(received.info.src_address);
        let rssi = RssiDbm::from_radio_raw(received.info.rx_control.rssi as u8);
        let now = Instant::now();

        let (sender_id, beacon) = match frame {
            protocol::DecodedFrame::Beacon(packet) => (packet.device_id, Some(packet)),
            protocol::DecodedFrame::Application(message) => {
                let recipient_valid = match message.recipient {
                    None => received.info.dst_address == BROADCAST_ADDRESS,
                    Some(recipient) => {
                        recipient == local_id && received.info.dst_address != BROADCAST_ADDRESS
                    }
                };
                if !recipient_valid {
                    record_invalid(runtime);
                    continue;
                }
                (message.sender, None)
            }
        };

        if sender_id == local_id {
            continue;
        }

        diagnostics::record_network_rx_packet();
        let outcome = with_state(|state| state.record_receive(sender_id, mac, rssi, now.as_millis()));
        let is_new = outcome.is_some_and(|outcome| outcome.is_new);
        if outcome.is_some_and(|outcome| outcome.evicted) {
            diagnostics::record_network_peer_eviction();
        }

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

        match frame {
            protocol::DecodedFrame::Beacon(packet) => {
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
            }
            protocol::DecodedFrame::Application(message) => {
                let _ = runtime.push_incoming(message.sender, message.recipient, message.payload);
            }
        }

        publish_snapshot(runtime, now);
    }
}

fn record_invalid(runtime: Runtime) {
    diagnostics::record_network_rx_invalid();
    let _ = with_state(NetworkState::record_invalid_receive);
    publish_snapshot(runtime, Instant::now());
}
