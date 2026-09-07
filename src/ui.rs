//! CPU0 presentation coordinator.
//!
//! UI state is deliberately small: one active view, one navigation touch
//! gesture, and fixed-buffer render calls. No widget tree or retained runtime
//! allocator is involved.

use core::fmt::Write as _;

use arrayvec::ArrayString;
use embassy_time::Instant;

use crate::{
    models::{AppModel, ViewId},
    network,
    screen::Screen,
    touch,
};

const NAV_WIDTH: u16 = 44;
const NAV_BUTTON_HEIGHT: u16 = 48;

#[derive(Clone, Copy)]
struct TouchState {
    pressed: bool,
    candidate: Option<ViewId>,
}

impl TouchState {
    const fn new() -> Self {
        Self {
            pressed: false,
            candidate: None,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct NavigationChange {
    pub from: ViewId,
    pub to: ViewId,
}

fn navigation_view_at(point: touch::TouchPoint) -> Option<ViewId> {
    if point.x >= NAV_WIDTH {
        return None;
    }

    let index = usize::from(point.y / NAV_BUTTON_HEIGHT);
    ViewId::ALL.get(index).copied()
}

fn dispatch_touch_input(model: &AppModel, state: &mut TouchState) {
    while let Some(edge) = touch::try_take_edge() {
        match edge {
            touch::TouchEdge::Pressed(point) => {
                state.pressed = true;
                state.candidate = navigation_view_at(point);
            }
            touch::TouchEdge::Released(point) => {
                if state.pressed {
                    if let Some(view) = state.candidate {
                        if navigation_view_at(point) == Some(view) {
                            model.request_view(view);
                        }
                    }
                }

                state.pressed = false;
                state.candidate = None;
            }
        }
    }

    if state.pressed {
        if let Some(point) = touch::take_latest_point() {
            if navigation_view_at(point) != state.candidate {
                state.candidate = None;
            }
        }
    } else {
        let _ = touch::take_latest_point();
    }
}

fn render_network_status(screen: &mut Screen, snapshot: &network::Snapshot) {
    let mut text = ArrayString::<768>::new();
    let status = match snapshot.status {
        network::Status::Starting => "STARTING",
        network::Status::Ready => "READY - WAITING FOR PEER",
        network::Status::PeerPresent => "PEER CONNECTED",
        network::Status::Fault => "RADIO FAULT",
    };

    let _ = writeln!(&mut text, "ESP-NOW  {}", status);
    let _ = writeln!(&mut text, "DEVICE  {}", snapshot.local_id);
    let _ = writeln!(
        &mut text,
        "CHANNEL {}   PEERS {}/{}",
        snapshot.channel,
        snapshot.peer_count,
        network::MAX_PEERS
    );
    let _ = writeln!(
        &mut text,
        "TX {}   RX {}   TXERR {}   INVALID {}",
        snapshot.tx_packets,
        snapshot.rx_packets,
        snapshot.tx_errors,
        snapshot.rx_invalid
    );
    let _ = writeln!(&mut text);

    if snapshot.peer_count == 0 {
        let _ = writeln!(&mut text, "Waiting for another Hack and Hike device...");
        let _ = writeln!(&mut text, "Flash this build to device #2.");
    } else {
        for (index, peer) in snapshot.peers.iter().filter(|peer| peer.present).enumerate() {
            let _ = writeln!(&mut text, "PEER {}  {}", index + 1, peer.device_id);
            let _ = writeln!(
                &mut text,
                "RSSI {} dBm  age {} ms  RX {}",
                peer.rssi_dbm,
                peer.age_ms,
                peer.rx_packets
            );
            let _ = writeln!(
                &mut text,
                "MAC {:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
                peer.mac[0],
                peer.mac[1],
                peer.mac[2],
                peer.mac[3],
                peer.mac[4],
                peer.mac[5]
            );
            let _ = writeln!(&mut text);
        }
    }

    screen.render_log(text.as_str());
}

pub struct Ui {
    model: AppModel,
    touch: TouchState,
    presented_view: ViewId,
}

impl Ui {
    pub fn new(model: AppModel) -> Self {
        let presented_view = model.active_view();
        Self {
            model,
            touch: TouchState::new(),
            presented_view,
        }
    }

    pub fn render_initial(&self, screen: &mut Screen) {
        screen.render_view_shell(self.presented_view);
    }

    pub fn prepare_frame(&mut self, now: Instant) -> Option<NavigationChange> {
        dispatch_touch_input(&self.model, &mut self.touch);
        self.model.update(now);

        let requested = self.model.active_view();
        (requested != self.presented_view).then_some(NavigationChange {
            from: self.presented_view,
            to: requested,
        })
    }

    pub fn apply_navigation(&mut self, change: NavigationChange, screen: &mut Screen) {
        self.presented_view = change.to;
        screen.render_view_shell(change.to);
    }

    pub fn render(&self, screen: &mut Screen) {
        match self.presented_view {
            ViewId::Network => {
                if let Some(snapshot) = self.model.take_network_display() {
                    render_network_status(screen, &snapshot);
                }
            }
            ViewId::Microphone => {
                if let Some(frame) = self.model.take_waveform_frame() {
                    screen.render_waveform(&frame);
                }
            }
            ViewId::Imu => {
                if let Some(imu) = self.model.take_imu_display() {
                    screen.render_imu(&imu);
                }
            }
            ViewId::Log => {
                let _ = self.model.with_log_text(|text| screen.render_log(text));
            }
            ViewId::Sound => {}
        }
    }
}
