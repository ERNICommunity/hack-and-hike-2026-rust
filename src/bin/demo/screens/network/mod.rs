//! The ESP-NOW demo. It broadcasts a ping every second, answers pings with
//! pongs and lists the peers in range.
//!
//! ESP-NOW is Espressif's protocol for short Wi-Fi messages between boards,
//! without a router. A peer is another board that this board has heard
//! recently.

use core::fmt::{self, Write as _};

use arrayvec::ArrayString;
use embassy_time::{Duration, Instant};
use embedded_graphics::primitives::Rectangle;
use hack_and_hike::{
    capabilities::{
        display::Surface,
        network::{self, DecodeError, Message, Network, SendError},
    },
    ui::{
        Canvas,
        common::{self, Lines},
        gui, theme,
    },
};
use serde::{Deserialize, Serialize};

use crate::{layout, screens::Screen};

// The layout file becomes Rust code at compile time: a `...App` struct with a
// `build` function and one `WidgetId` for each named node.
/// The widgets generated from `network.kdl`: the title and the two text slots.
mod generated {
    use embedded_gui::prelude::*;
    embedded_gui::include_gui!("src/bin/demo/screens/network/network.kdl");
}

/// Room for widgets in this screen's GUI context: the KDL nodes plus the
/// widgets added in code.
const NODES: usize = 16;
const _: () = assert!(generated::NetworkApp::WIDTH == layout::CONTENT_SIZE.width);
const _: () = assert!(generated::NetworkApp::HEIGHT == layout::CONTENT_SIZE.height);
/// How often the screen checks for a new snapshot (peer table and counters)
/// and whether a ping is due. Messages are read on every call of `update`, not
/// only once in this period. The receive queue holds only four messages, and
/// all boards in range answer a ping at the same time.
const UPDATE_PERIOD: Duration = Duration::from_millis(50);
/// How often this board broadcasts a ping.
const PING_PERIOD: Duration = Duration::from_secs(1);

/// The application's own message type. The network only moves the bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum DemoMessage {
    /// "Is anyone there?", broadcast about once a second.
    Ping {
        /// The number of this ping on the sending board: 1 for the first
        /// ping.
        sequence: u32,
    },
    /// The answer to a ping, sent only to the board that sent the ping.
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
    /// Pings that this board queued for broadcast.
    pings_sent: u32,
    /// Pings from other boards. The screen answers each one with a pong.
    pings_received: u32,
    /// Pongs from other boards: answers to this board's pings.
    pongs_received: u32,
    /// Pings or pongs that `broadcast` or `send_to` refused for a reason
    /// other than a full queue, for example an unknown peer. The network
    /// counts a full queue itself, in `Snapshot::tx_queue_full`.
    send_errors: u32,
    /// Messages of this application's kind whose bytes did not decode.
    decode_errors: u32,
    /// Messages of other applications on the same radio channel.
    other_kinds: u32,
}

/// The network screen: it pings, answers pings and lists peers.
pub(crate) struct NetworkScreen {
    /// The network handle: it sends, receives and reports the peer table.
    network: Network,
    /// What was sent and received since boot.
    counters: Counters,
    /// Sequence number of the next ping.
    next_sequence: u32,
    /// The newest snapshot with a change that the screen shows. `None` until
    /// the radio task publishes the first snapshot.
    snapshot: Option<network::Snapshot>,
    /// When `update` last checked the snapshot and the ping timer. It checks
    /// them at most once every `UPDATE_PERIOD`.
    last_update: Instant,
    /// When this board last tried to send a ping. It tries once every
    /// `PING_PERIOD`.
    last_ping: Instant,
    /// The widget tree built from `network.kdl`, drawn under the text.
    gui: &'static mut gui::Context<NODES>,
    /// Where the status lines go.
    summary: Rectangle,
    /// Where the peer list goes.
    peers: Rectangle,
    /// Whether the screen needs a redraw.
    dirty: bool,
}

impl NetworkScreen {
    /// Build the layout. The first ping is sent about one second later.
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

    /// Read every waiting message. Count it, and answer each ping with a pong.
    fn handle_messages(&mut self) {
        while let Some(message) = self.network.next_message() {
            match message.decode::<DemoMessage>() {
                Ok(DemoMessage::Ping { sequence }) => {
                    self.counters.pings_received += 1;
                    let pong = DemoMessage::Pong { sequence };
                    if let Err(error) = self.network.send_to(message.sender, &pong) {
                        self.count_send_error(error);
                    }
                }
                Ok(DemoMessage::Pong { .. }) => self.counters.pongs_received += 1,
                Err(DecodeError::WrongKind) => self.counters.other_kinds += 1,
                Err(DecodeError::Malformed) => self.counters.decode_errors += 1,
            }
            self.dirty = true;
        }
    }

    /// Broadcast the next ping. The sequence number advances only when the
    /// ping was queued.
    fn send_ping(&mut self) {
        let ping = DemoMessage::Ping {
            sequence: self.next_sequence,
        };
        match self.network.broadcast(&ping) {
            Ok(()) => {
                self.next_sequence += 1;
                self.counters.pings_sent += 1;
            }
            Err(error) => self.count_send_error(error),
        }
        self.dirty = true;
    }

    /// Count a failed send. The network already counts a full queue in
    /// [`network::Snapshot::tx_queue_full`], so it is not counted twice.
    fn count_send_error(&mut self, error: SendError) {
        if error != SendError::QueueFull {
            self.counters.send_errors += 1;
        }
    }
}

impl Screen for NetworkScreen {
    fn enter(&mut self) {
        self.dirty = true;
    }

    fn update(&mut self, now: Instant) {
        self.handle_messages();
        if now - self.last_update < UPDATE_PERIOD {
            return;
        }
        self.last_update = now;

        let snapshot = self.network.snapshot();
        // The revision does not change when only the queue counters change.
        // The screen shows them too, so compare them as well.
        let shown = |s: network::Snapshot| (s.revision, s.tx_queue_full, s.rx_queue_full);
        if snapshot.map(shown) != self.snapshot.map(shown) {
            self.snapshot = snapshot;
            self.dirty = true;
        }
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

/// The status lines: the radio state, this board, the channel and the
/// counters.
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
    // receives with a pong. So the number of pings sent grows every second.
    // With another board in range, pings received and pongs back grow too.
    //
    // "lost" adds up the messages that did not fit the send queue or the
    // receive queue, and the sends that this screen counted as errors. "bad"
    // adds up the messages of this kind that did not decode, and the received
    // frames that are not valid for this protocol or are for another board.
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
        snapshot.tx_queue_full + snapshot.rx_queue_full + counters.send_errors,
        counters.decode_errors + snapshot.rx_invalid
    );
    lines.line(&text, theme::DARK_GRAY);

    text.clear();
    let _ = write!(text, "Other apps' messages {}", counters.other_kinds);
    lines.line(&text, theme::DARK_GRAY);
}

/// A header and one line for each peer in range, as many as fit `area`. When
/// there is no peer, a hint instead.
///
/// RSSI (received signal strength indicator) is the signal strength of the
/// last frame from the peer, in dBm.
fn draw_peers(canvas: &mut Canvas, area: Rectangle, snapshot: Option<&network::Snapshot>) {
    let mut lines = Lines::new(canvas, area.top_left);
    let peers = snapshot.into_iter().flat_map(network::Snapshot::peers);
    let mut text = ArrayString::<48>::new();
    let mut listed = 0;

    // The rows that fit: one header line plus one line for each peer.
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

/// Signal strength, for example `-42dBm`. dBm is decibels relative to one
/// milliwatt. A value closer to zero means a stronger signal.
struct Dbm(i8);

impl fmt::Display for Dbm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // `pad` applies the width of the format string, such as `{:>6}`. A
        // plain `write!` would ignore it.
        let mut text = ArrayString::<12>::new();
        write!(text, "{}dBm", self.0)?;
        f.pad(&text)
    }
}

/// How long a peer has been running, for example `12m 03s`. `None` shows as
/// "unknown": the peer has not sent a beacon yet, only application
/// messages. (A beacon is the short "I am here" frame that every board sends
/// four times a second.)
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
