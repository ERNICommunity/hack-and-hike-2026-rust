//! ESP-NOW peer overview.
//!
//! KDL owns the summary/table regions. Formatting and the dense peer table remain
//! specific to the Network view and render with a native-resolution font into
//! those regions.

use core::fmt::Write as _;

use arrayvec::ArrayString;
use embedded_gui::prelude::*;

use crate::{
    services::{display::Display, network},
    support::memory::storage,
};

use super::super::gui::GuiSurface;
use super::common;

mod generated {
    use embedded_gui::prelude::*;
    embedded_gui::include_gui!("src/app/ui/views/network/network.kdl");
}

const NODE_CAPACITY: usize = 8;
const TEXT_CAPACITY: usize = 4;
const EVENT_CAPACITY: usize = 2;

type Context = GuiContext<'static, NODE_CAPACITY, TEXT_CAPACITY, EVENT_CAPACITY>;

#[derive(Clone, Copy)]
struct Geometry {
    title: Rect,
    summary: Rect,
    peers: Rect,
}

pub(in crate::app::ui) struct View {
    gui: &'static mut Context,
    geometry: Geometry,
}

impl View {
    pub(in crate::app::ui) fn new() -> Self {
        let gui = storage::leaked_value_with(|| Context::new(Rect::new(0, 0, 276, 240)));
        let app = generated::NetworkApp::build(gui)
            .expect("network KDL exceeds embedded-gui fixed capacities");
        Self {
            geometry: Geometry {
                title: required_rect(gui, app.widgets.title_slot, "network title"),
                summary: required_rect(gui, app.widgets.summary_slot, "network summary"),
                peers: required_rect(gui, app.widgets.peers_slot, "network peers"),
            },
            gui,
        }
    }

    pub(in crate::app::ui) fn present_shell(
        &mut self,
        surface: &mut GuiSurface,
        display: &mut Display,
    ) {
        let geometry = self.geometry;
        surface.present_with_overlay(display, self.gui, move |frame| {
            common::draw_title(
                frame,
                "NETWORK",
                geometry.title.x,
                geometry.title.y,
                common::dark_blue(),
            );
            common::draw_body(
                frame,
                "Waiting for ESP-NOW state...",
                geometry.summary.x,
                geometry.summary.y,
                common::dark_gray(),
            );
        });
    }

    pub(in crate::app::ui) fn present(
        &mut self,
        surface: &mut GuiSurface,
        display: &mut Display,
        snapshot: &network::Snapshot,
    ) {
        let geometry = self.geometry;
        surface.present_with_overlay(display, self.gui, move |frame| {
            draw_network(frame, geometry, snapshot);
        });
    }
}

fn draw_network(
    frame: &mut super::super::gui::GuiFramebuffer,
    geometry: Geometry,
    snapshot: &network::Snapshot,
) {
    common::draw_title(
        frame,
        "NETWORK",
        geometry.title.x,
        geometry.title.y,
        common::dark_blue(),
    );

    let status = match snapshot.status {
        network::Status::Starting => "ESP-NOW  STARTING",
        network::Status::Ready => "ESP-NOW  WAITING FOR PEER",
        network::Status::PeerPresent => "ESP-NOW  PEER CONNECTED",
        network::Status::Fault => "ESP-NOW  RADIO FAULT",
    };
    common::draw_body(
        frame,
        status,
        geometry.summary.x,
        geometry.summary.y,
        common::black(),
    );

    let mut line = ArrayString::<64>::new();
    let _ = write!(&mut line, "ID {}", snapshot.local_id);
    common::draw_body(
        frame,
        line.as_str(),
        geometry.summary.x,
        geometry.summary.y + common::BODY_LINE_HEIGHT,
        common::black(),
    );

    line.clear();
    let _ = write!(
        &mut line,
        "CH {}  PEERS {}/{}",
        snapshot.channel,
        snapshot.peer_count(),
        network::MAX_PEERS,
    );
    common::draw_body(
        frame,
        line.as_str(),
        geometry.summary.x,
        geometry.summary.y + common::BODY_LINE_HEIGHT * 2,
        common::black(),
    );

    line.clear();
    let _ = write!(
        &mut line,
        "TX{} RX{} E{} B{} V{}",
        snapshot.tx_packets,
        snapshot.rx_packets,
        snapshot.tx_errors,
        snapshot.rx_invalid,
        snapshot.peer_evictions,
    );
    common::draw_body(
        frame,
        line.as_str(),
        geometry.summary.x,
        geometry.summary.y + common::BODY_LINE_HEIGHT * 3,
        common::dark_gray(),
    );

    if snapshot.peer_count() == 0 {
        common::draw_body(
            frame,
            "No peers in range",
            geometry.peers.x,
            geometry.peers.y,
            common::dark_gray(),
        );
        return;
    }

    let max_rows = geometry.peers.h as i32 / common::BODY_LINE_HEIGHT;
    for (index, peer) in snapshot.peers().take(max_rows as usize).enumerate() {
        line.clear();
        let _ = write!(
            &mut line,
            "P{:02} {} {:>4}dB {:>4}ms",
            index + 1,
            peer.device_id,
            peer.rssi_dbm,
            peer.age_ms.min(9999),
        );
        common::draw_body(
            frame,
            line.as_str(),
            geometry.peers.x,
            geometry.peers.y + index as i32 * common::BODY_LINE_HEIGHT,
            common::black(),
        );
    }
}

fn required_rect(gui: &Context, id: WidgetId, name: &'static str) -> Rect {
    gui.absolute_rect(id)
        .unwrap_or_else(|| panic!("{name} layout missing"))
}
