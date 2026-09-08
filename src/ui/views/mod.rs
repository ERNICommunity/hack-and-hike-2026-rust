//! Semantic view dispatch.
//!
//! Each screen owns its presentation code in a matching module. Reusable drawing
//! primitives live in `common`; view-specific helpers remain private to the view
//! that uses them.

mod common;
mod imu;
mod log;
mod microphone;
mod network;
mod speaker;

use crate::{
    display::Display,
    models::{ImuDisplay, ViewId},
    network as network_service,
    waveform::WaveformFrame,
};

use super::framebuffer::ContentFramebuffer;

pub(crate) fn render_shell(frame: &mut ContentFramebuffer, view: ViewId) {
    match view {
        ViewId::Network => network::render_shell(frame),
        ViewId::Imu => imu::render_shell(frame),
        ViewId::Microphone => microphone::render_shell(frame),
        ViewId::Speaker => speaker::render_shell(frame),
        ViewId::Log => log::render_shell(frame),
    }
}

pub(crate) fn render_network(
    frame: &mut ContentFramebuffer,
    snapshot: &network_service::Snapshot,
) {
    network::render(frame, snapshot);
}

pub(crate) fn render_imu(frame: &mut ContentFramebuffer, display: &ImuDisplay) {
    imu::render(frame, display);
}

pub(crate) fn render_microphone(display: &mut Display, frame: &WaveformFrame) {
    microphone::render(display, frame);
}

pub(crate) fn render_log(frame: &mut ContentFramebuffer, contents: &str) {
    log::render(frame, contents);
}
