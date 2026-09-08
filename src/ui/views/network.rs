//! ESP-NOW peer overview view.

use core::fmt::Write as _;

use arrayvec::ArrayString;

use crate::network;

use super::{common, super::framebuffer::ContentFramebuffer};

pub(super) fn render_shell(frame: &mut ContentFramebuffer) {
    common::render_placeholder(frame, "NETWORK", "Peer communication");
}

pub(super) fn render(frame: &mut ContentFramebuffer, snapshot: &network::Snapshot) {
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
            let _ = writeln!(
                &mut text,
                "P{:02} {} {:>4}dBm {:>4}ms RX{}",
                index + 1,
                peer.device_id,
                peer.rssi_dbm,
                peer.age_ms.min(9999),
                peer.rx_packets,
            );
        }
    }

    common::render_text_page(frame, text.as_str());
}
