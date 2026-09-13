//! ESP-NOW demo: broadcasts a ping every second, answers pings with pongs and
//! lists the peers in range.

use core::fmt::{self, Write as _};

use arrayvec::ArrayString;
use embassy_time::{Duration, Instant};
use embedded_graphics::prelude::Point;
use embedded_gui::Rect;
use hack_and_hike::{
    capabilities::{
        display::Surface,
        network::{self, DecodeError, Message, Network},
    },
    ui::{
        common::{self, Lines},
        gui::{self, GuiFramebuffer, GuiSurface},
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

impl Message for DemoMessage {
    const NAME: &'static str = "hack-and-hike.demo.ping-pong";
}

#[derive(Clone, Copy, Default)]
struct Counters {
    pings_sent: u32,
    pings_received: u32,
    pongs_received: u32,
    send_errors: u32,
    decode_errors: u32,
    /// Messages of other applications sharing the channel.
    other_kinds: u32,
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
        while let Some(message) = self.network.next_message() {
            match message.decode::<DemoMessage>() {
                Ok(DemoMessage::Ping { sequence }) => {
                    self.counters.pings_received += 1;
                    let pong = DemoMessage::Pong { sequence };
                    if self.network.send_to(message.sender, &pong).is_err() {
                        self.counters.send_errors += 1;
                    }
                }
                Ok(DemoMessage::Pong { .. }) => self.counters.pongs_received += 1,
                Err(DecodeError::WrongKind) => self.counters.other_kinds += 1,
                Err(DecodeError::Malformed) => self.counters.decode_errors += 1,
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

        let snapshot = self.network.snapshot();
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
            draw_summary(frame, summary, counters, snapshot.as_ref());
            draw_peers(frame, peers, snapshot.as_ref());
        });
    }
}

fn draw_summary(
    frame: &mut GuiFramebuffer,
    area: Rect,
    counters: Counters,
    snapshot: Option<&network::Snapshot>,
) {
    let mut lines = Lines::new(frame, Point::new(area.x, area.y));
    let mut text = ArrayString::<48>::new();

    let Some(snapshot) = snapshot else {
        lines.line("ESP-NOW: starting up", theme::CHARCOAL);
        return;
    };

    let peer_count = snapshot.peer_count();
    let _ = match snapshot.status {
        network::Status::Starting => write!(text, "ESP-NOW: starting up"),
        network::Status::Ready => write!(text, "ESP-NOW: ready, no peers yet"),
        network::Status::PeerPresent if peer_count == 1 => {
            write!(text, "ESP-NOW: 1 peer in range")
        }
        network::Status::PeerPresent => write!(text, "ESP-NOW: {peer_count} peers in range"),
        network::Status::Fault => write!(text, "ESP-NOW: radio fault"),
    };
    lines.line(&text, theme::CHARCOAL);

    text.clear();
    let _ = write!(text, "This device {}", snapshot.local_id);
    lines.line(&text, theme::CHARCOAL);

    text.clear();
    let _ = write!(
        text,
        "Channel {}, room for {} peers",
        snapshot.channel,
        network::MAX_PEERS
    );
    lines.line(&text, theme::DARK_GRAY);

    // This screen broadcasts a ping every second and answers every ping it
    // receives with a pong, so these three numbers should keep climbing.
    text.clear();
    let _ = write!(
        text,
        "Pings {} sent, {} received",
        counters.pings_sent, counters.pings_received
    );
    lines.line(&text, theme::CHARCOAL);

    text.clear();
    let _ = write!(
        text,
        "Pongs {} back, {} lost, {} bad",
        counters.pongs_received,
        counters.send_errors + snapshot.tx_queue_full + snapshot.rx_queue_full,
        counters.decode_errors + snapshot.rx_invalid
    );
    lines.line(&text, theme::DARK_GRAY);

    text.clear();
    let _ = write!(text, "Other apps' messages {}", counters.other_kinds);
    lines.line(&text, theme::DARK_GRAY);
}

fn draw_peers(frame: &mut GuiFramebuffer, area: Rect, snapshot: Option<&network::Snapshot>) {
    let mut lines = Lines::new(frame, Point::new(area.x, area.y));
    let peers = snapshot.into_iter().flat_map(network::Snapshot::peers);
    let mut text = ArrayString::<48>::new();
    let mut listed = 0;

    // One header line plus one line per peer.
    let rows = (area.h as i32 / common::BODY_LINE_HEIGHT).max(1) as usize;
    lines.line("PEER ADDRESS      RSSI    UPTIME", theme::DARK_GRAY);
    for peer in peers.take(rows - 1) {
        text.clear();
        let _ = write!(
            text,
            "{} {:>6}  {}",
            peer.id,
            Dbm(peer.rssi_dbm),
            Uptime(peer.uptime_ms)
        );
        lines.line(&text, theme::CHARCOAL);
        listed += 1;
    }

    if listed == 0 {
        lines.line("", theme::CHARCOAL);
        lines.line("No other boards are broadcasting.", theme::DARK_GRAY);
        lines.line("Flash this application to a second", theme::DARK_GRAY);
        lines.line("board and it will appear here.", theme::DARK_GRAY);
    }
}

/// Signal strength, for example `-42dBm`.
struct Dbm(i8);

impl fmt::Display for Dbm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}dBm", self.0)
    }
}

/// How long a peer has been running, for example `12m 03s`. A peer that has
/// only sent application messages has not reported an uptime yet.
struct Uptime(Option<u32>);

impl fmt::Display for Uptime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Some(milliseconds) = self.0 else {
            return write!(f, "unknown");
        };
        let seconds = milliseconds / 1_000;
        let (hours, minutes, seconds) = (seconds / 3_600, (seconds / 60) % 60, seconds % 60);
        if hours > 0 {
            write!(f, "{hours}h {minutes:02}m")
        } else if minutes > 0 {
            write!(f, "{minutes}m {seconds:02}s")
        } else {
            write!(f, "{seconds}s")
        }
    }
}
