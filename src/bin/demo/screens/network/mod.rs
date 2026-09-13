//! ESP-NOW demo: broadcasts a ping every second, answers pings with pongs and
//! lists the peers in range.

use core::fmt::Write as _;

use arrayvec::ArrayString;
use embassy_time::{Duration, Instant};
use embedded_graphics::prelude::Point;
use embedded_gui::Rect;
use hack_and_hike::{
    capabilities::{
        display::Surface,
        network::{self, Network},
    },
    ui::{
        common::{self, Lines},
        gui::{self, GuiSurface},
        theme,
    },
};
use serde::{Deserialize, Serialize};

use crate::{layout, screens::Screen};

mod generated {
    use embedded_gui::prelude::*;
    embedded_gui::include_gui!("src/bin/demo/screens/network/network.kdl");
}

const NODES: usize = 16;
const UPDATE_PERIOD: Duration = Duration::from_millis(50);
const PING_PERIOD: Duration = Duration::from_secs(1);

/// The application's own message type; the network only moves bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum DemoMessage {
    Ping { sequence: u32 },
    Pong { sequence: u32 },
}

#[derive(Clone, Copy, Default)]
struct Counters {
    pings_sent: u32,
    pings_received: u32,
    pongs_received: u32,
    send_errors: u32,
    decode_errors: u32,
}

pub(crate) struct NetworkScreen {
    network: Network,
    counters: Counters,
    next_sequence: u32,
    snapshot: Option<network::Snapshot>,
    last_update: Instant,
    last_ping: Instant,
    gui: &'static mut gui::Context<NODES>,
    summary: Rect,
    peers: Rect,
    dirty: bool,
}

impl NetworkScreen {
    pub(crate) fn new(network: Network) -> Self {
        let gui = gui::context::<NODES>(layout::CONTENT_WIDTH, layout::CONTENT_HEIGHT);
        let app = generated::NetworkApp::build(gui).expect("network.kdl fits the GUI capacities");
        let now = Instant::now();
        Self {
            network,
            counters: Counters::default(),
            next_sequence: 1,
            snapshot: None,
            last_update: now,
            last_ping: now,
            summary: gui::slot(gui, app.widgets.summary),
            peers: gui::slot(gui, app.widgets.peers),
            gui,
            dirty: true,
        }
    }

    fn handle_messages(&mut self) {
        while let Some(message) = self.network.receive() {
            match message.decode::<DemoMessage>() {
                Ok(DemoMessage::Ping { sequence }) => {
                    self.counters.pings_received += 1;
                    let pong = DemoMessage::Pong { sequence };
                    if self.network.send_to(message.sender, &pong).is_err() {
                        self.counters.send_errors += 1;
                    }
                }
                Ok(DemoMessage::Pong { .. }) => self.counters.pongs_received += 1,
                Err(_) => self.counters.decode_errors += 1,
            }
            self.dirty = true;
        }
    }

    fn send_ping(&mut self) {
        let ping = DemoMessage::Ping {
            sequence: self.next_sequence,
        };
        match self.network.broadcast(&ping) {
            Ok(()) => {
                self.next_sequence += 1;
                self.counters.pings_sent += 1;
            }
            Err(_) => self.counters.send_errors += 1,
        }
        self.dirty = true;
    }
}

impl Screen for NetworkScreen {
    fn enter(&mut self) {
        self.dirty = true;
    }

    fn update(&mut self, now: Instant) {
        if now - self.last_update < UPDATE_PERIOD {
            return;
        }
        self.last_update = now;

        let snapshot = self.network.snapshot().copied();
        if snapshot.map(|s| s.revision) != self.snapshot.map(|s| s.revision) {
            self.snapshot = snapshot;
            self.dirty = true;
        }
        self.handle_messages();
        if now - self.last_ping >= PING_PERIOD {
            self.last_ping = now;
            self.send_ping();
        }
    }

    fn present(&mut self, gui: &mut GuiSurface, surface: &mut Surface<'_>) {
        if !self.dirty {
            return;
        }
        self.dirty = false;

        let (summary, peers, counters, snapshot) =
            (self.summary, self.peers, self.counters, self.snapshot);
        gui.present(surface, self.gui, |frame| {
            let mut text = ArrayString::<64>::new();
            let mut lines = Lines::new(frame, Point::new(summary.x, summary.y));
            let Some(snapshot) = snapshot else {
                lines.line("ESP-NOW  STARTING", theme::CHARCOAL);
                return;
            };

            let status = match snapshot.status {
                network::Status::Starting => "ESP-NOW  STARTING",
                network::Status::Ready => "ESP-NOW  WAITING FOR PEER",
                network::Status::PeerPresent => "ESP-NOW  PEER CONNECTED",
                network::Status::Fault => "ESP-NOW  RADIO FAULT",
            };
            lines.line(status, theme::CHARCOAL);
            let _ = write!(text, "ID {}", snapshot.local_id);
            lines.line(&text, theme::CHARCOAL);
            text.clear();
            let _ = write!(
                text,
                "CH {}  PEERS {}/{}",
                snapshot.channel,
                snapshot.peer_count(),
                network::MAX_PEERS
            );
            lines.line(&text, theme::CHARCOAL);
            text.clear();
            let _ = write!(
                text,
                "M {}/{}/{} E{}/{} Q{}/{}",
                counters.pings_sent,
                counters.pings_received,
                counters.pongs_received,
                counters.send_errors,
                counters.decode_errors,
                snapshot.tx_queue_full,
                snapshot.rx_queue_full
            );
            lines.line(&text, theme::DARK_GRAY);

            let mut lines = Lines::new(frame, Point::new(peers.x, peers.y));
            if snapshot.peer_count() == 0 {
                lines.line("No peers in range", theme::DARK_GRAY);
                return;
            }
            let max_rows = (peers.h as i32 / common::BODY_LINE_HEIGHT).max(0) as usize;
            for (index, peer) in snapshot.peers().take(max_rows).enumerate() {
                text.clear();
                let _ = write!(
                    text,
                    "P{:02} {} {:>4}dB A{} E{}",
                    index + 1,
                    peer.id,
                    peer.rssi_dbm,
                    peer.age_ms.min(9999),
                    peer.expires_in_ms.min(9999)
                );
                lines.line(&text, theme::CHARCOAL);
            }
        });
    }
}
