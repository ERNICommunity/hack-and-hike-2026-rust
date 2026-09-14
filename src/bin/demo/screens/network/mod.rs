//! ESP-NOW demo: broadcasts a ping every second, answers pings with pongs and
//! lists the peers in range.

use core::fmt::{self, Write as _};

use arrayvec::ArrayString;
use embassy_time::{Duration, Instant};
use embedded_graphics::primitives::Rectangle;
use hack_and_hike::{
    capabilities::{
        display::Surface,
        network::{self, DecodeError, Message, Network},
    },
    ui::{
        Canvas,
        common::{self, Lines},
        gui, theme,
    },
};
use serde::{Deserialize, Serialize};

use crate::{layout, screens::Screen};

// The layout file becomes Rust at compile time: a `...App` struct with a
// `build` function and one `WidgetId` per named node.
mod generated {
    use embedded_gui::prelude::*;
    embedded_gui::include_gui!("src/bin/demo/screens/network/network.kdl");
}

/// Room for widgets in this screen's GUI context: the KDL nodes plus the
/// widgets added in code.
const NODES: usize = 16;
const _: () = assert!(generated::NetworkApp::WIDTH == layout::CONTENT_SIZE.width);
const _: () = assert!(generated::NetworkApp::HEIGHT == layout::CONTENT_SIZE.height);
/// How often the screen checks for messages and a new peer table.
const UPDATE_PERIOD: Duration = Duration::from_millis(50);
/// How often this board broadcasts a ping.
const PING_PERIOD: Duration = Duration::from_secs(1);

/// The application's own message type; the network only moves bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum DemoMessage {
    /// "Is anyone there?", broadcast every second.
    Ping {
        /// Counts the pings of the sending board.
        sequence: u32,
    },
    /// The answer to a ping, sent back to the pinging board only.
    Pong {
        /// The sequence number of the ping it answers.
        sequence: u32,
    },
}

impl Message for DemoMessage {
    const NAME: &'static str = "hack-and-hike.demo.ping-pong";
}

/// What this screen has sent and received, shown in the summary.
#[derive(Clone, Copy, Default)]
struct Counters {
    pings_sent: u32,
    pings_received: u32,
    pongs_received: u32,
    /// Pings or pongs that could not be queued.
    send_errors: u32,
    /// Messages of our kind with bytes that did not decode.
    decode_errors: u32,
    /// Messages of other applications sharing the channel.
    other_kinds: u32,
}

/// The network screen: it pings, answers pings and lists peers.
pub(crate) struct NetworkScreen {
    network: Network,
    counters: Counters,
    /// Sequence number of the next ping.
    next_sequence: u32,
    /// The peer table last shown.
    snapshot: Option<network::Snapshot>,
    last_update: Instant,
    last_ping: Instant,
    gui: &'static mut gui::Context<NODES>,
    /// Where the status lines go.
    summary: Rectangle,
    /// Where the peer list goes.
    peers: Rectangle,
    /// Whether the screen needs a redraw.
    dirty: bool,
}

impl NetworkScreen {
    /// Build the layout; the first ping goes out a second later.
    pub(crate) fn new(network: Network) -> Self {
        let gui = gui::context::<NODES>(layout::CONTENT_SIZE.width, layout::CONTENT_SIZE.height);
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

    /// Read every waiting message: count it, and answer pings with a pong.
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

    /// Broadcast the next ping.
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

    fn present(&mut self, canvas: &mut Canvas, surface: &mut Surface<'_>) {
        if !self.dirty {
            return;
        }
        self.dirty = false;

        canvas.clear(theme::WHITE);
        gui::render(self.gui, canvas);
        draw_summary(canvas, self.summary, self.counters, self.snapshot.as_ref());
        draw_peers(canvas, self.peers, self.snapshot.as_ref());
        canvas.show(surface);
    }
}

/// The status lines: radio state, this board, channel and counters.
fn draw_summary(
    canvas: &mut Canvas,
    area: Rectangle,
    counters: Counters,
    snapshot: Option<&network::Snapshot>,
) {
    let mut lines = Lines::new(canvas, area.top_left);
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

/// One line per peer in range, as many as fit `area`, or a hint when there is
/// none.
fn draw_peers(canvas: &mut Canvas, area: Rectangle, snapshot: Option<&network::Snapshot>) {
    let mut lines = Lines::new(canvas, area.top_left);
    let peers = snapshot.into_iter().flat_map(network::Snapshot::peers);
    let mut text = ArrayString::<48>::new();
    let mut listed = 0;

    // One header line plus one line per peer.
    let rows = (area.size.height as i32 / common::BODY_LINE_HEIGHT).max(1) as usize;
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
        lines.skip();
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
