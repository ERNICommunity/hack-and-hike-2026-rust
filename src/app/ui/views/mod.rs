//! Concrete semantic view ownership.
//!
//! `Ui` owns navigation and coordination. This module only stores the concrete
//! view instances; each view keeps its own rendering and interaction mechanics.

mod camera;
mod common;
mod imu;
mod log;
mod microphone;
mod network;
mod settings;
mod speaker;

pub(super) use speaker::Action as SpeakerAction;

pub(super) struct Views {
    pub(super) network: network::View,
    pub(super) imu: imu::View,
    pub(super) microphone: microphone::View,
    pub(super) speaker: speaker::View,
    pub(super) camera: camera::View,
    pub(super) settings: settings::View,
    pub(super) log: log::View,
}

impl Views {
    pub(super) fn new() -> Self {
        Self {
            network: network::View::new(),
            imu: imu::View::new(),
            microphone: microphone::View::new(),
            speaker: speaker::View::new(),
            camera: camera::View::new(),
            settings: settings::View::new(),
            log: log::View::new(),
        }
    }
}
