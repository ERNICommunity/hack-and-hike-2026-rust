//! Monospaced Network and Log views.

use core::fmt::Write as _;

use arrayvec::ArrayString;
use embedded_graphics::{
    mono_font::{ascii::FONT_6X10, MonoTextStyle},
    prelude::*,
    text::{Baseline, Text},
};

use crate::network;

use super::super::{
    design,
    framebuffer::{color, ContentFramebuffer},
};

pub(super) fn render_network(frame: &mut ContentFramebuffer, snapshot: &network::Snapshot) {
    let mut text = ArrayString::<768>::new();
    let status = match snapshot.status {
        network::Status::Starting => "STARTING",
        network::Status::Ready => "READY - WAITING FOR PEER",
        network::Status::PeerPresent => "PEER CONNECTED",
        network::Status::Fault => "RADIO FAULT",
    };
    let peer_count = snapshot.peer_count();

    let _ = writeln!(&mut text, "ESP-NOW  {}", status);
    let _ = writeln!(&mut text, "DEVICE  {}", snapshot.local_id);
    let _ = writeln!(
        &mut text,
        "CHANNEL {}   PEERS {}/{}",
        snapshot.channel,
        peer_count,
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

    if peer_count == 0 {
        let _ = writeln!(&mut text, "Waiting for another Hack and Hike device...");
        let _ = writeln!(&mut text, "Flash this build to device #2.");
    } else {
        for (index, peer) in snapshot.peers().enumerate() {
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
            let _ = writeln!(&mut text, "MAC {}", peer.mac);
        }
    }

    render_text_page(frame, text.as_str());
}

pub(super) fn render_log(frame: &mut ContentFramebuffer, text: &str) {
    render_text_page(frame, trailing_lines(text, design::UI.text.visible_lines));
}

fn render_text_page(frame: &mut ContentFramebuffer, text: &str) {
    let spec = design::UI.text;
    frame.clear(spec.background);

    let style = MonoTextStyle::new(&FONT_6X10, color(spec.foreground));
    let mut y = spec.top;
    for line in text.lines().take(spec.visible_lines) {
        let _ = Text::with_baseline(
            line,
            Point::new(spec.x as i32, y as i32),
            style,
            Baseline::Top,
        )
        .draw(frame);
        y += spec.line_height;
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
