//! Monospaced Network and Log views.

use core::fmt::Write as _;

use arrayvec::ArrayString;
use embedded_graphics::{
    mono_font::{ascii::FONT_6X10, MonoTextStyle},
    prelude::*,
    text::{Baseline, Text},
};

use crate::{network, theme};

use super::super::{
    framebuffer::{color, ContentFramebuffer},
    layout,
};

pub(super) fn render_network(frame: &mut ContentFramebuffer, snapshot: &network::Snapshot) {
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
        "TX {} RX {} ERR {} BAD {} EVICT {}",
        snapshot.tx_packets,
        snapshot.rx_packets,
        snapshot.tx_errors,
        snapshot.rx_invalid,
        snapshot.peer_evictions,
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
                "RSSI {} dBm AGE {} ms RX {}",
                peer.rssi_dbm,
                peer.age_ms,
                peer.rx_packets,
            );
            let _ = writeln!(
                &mut text,
                "UPTIME {} ms CAP 0x{:08X}",
                peer.remote_uptime_ms,
                peer.capabilities,
            );
            let _ = writeln!(
                &mut text,
                "MAC {:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
                peer.mac[0], peer.mac[1], peer.mac[2], peer.mac[3], peer.mac[4], peer.mac[5]
            );
        }
    }

    render_text_page(frame, text.as_str());
}

pub(super) fn render_log(frame: &mut ContentFramebuffer, text: &str) {
    render_text_page(frame, trailing_lines(text, layout::TEXT_VISIBLE_LINES));
}

fn render_text_page(frame: &mut ContentFramebuffer, text: &str) {
    frame.clear(theme::WHITE_RGB565);

    let style = MonoTextStyle::new(&FONT_6X10, color(theme::BLACK_RGB565));
    let mut y = layout::TEXT_TOP;
    for line in text.lines().take(layout::TEXT_VISIBLE_LINES) {
        let _ = Text::with_baseline(line, Point::new(4, y), style, Baseline::Top).draw(frame);
        y += layout::TEXT_LINE_HEIGHT;
    }
}

fn trailing_lines(text: &str, line_count: usize) -> &str {
    if line_count == 0 || text.is_empty() {
        return "";
    }

    let bytes = text.as_bytes();
    let mut seen = 0usize;
    for index in (0..bytes.len()).rev() {
        if bytes[index] != b'\n' {
            continue;
        }

        seen += 1;
        if seen > line_count {
            return &text[index + 1..];
        }
    }

    text
}
