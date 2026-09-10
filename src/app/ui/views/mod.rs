//! Semantic view ownership.
//!
//! Every screen owns a KDL-backed fixed-capacity context in its matching module.
//! `Ui` decides which semantic view is active; this module only owns the concrete
//! view objects and exposes view-specific operations.

mod camera;
mod common;
mod imu;
mod log;
mod microphone;
mod network;
mod settings;
mod speaker;

use crate::{
    app::model::{ImuDisplay, SettingsDisplay, SpeakerDisplay, WaveformFrame},
    services::{
        camera as camera_service,
        display::{BrightnessPercent, Display},
        network as network_service,
    },
};

use super::{gui::GuiSurface, navigation::ContentPointer};

pub(super) use speaker::Action as SpeakerAction;

pub(super) struct Views {
    network: network::View,
    imu: imu::View,
    microphone: microphone::View,
    speaker: speaker::View,
    camera: camera::View,
    settings: settings::View,
    log: log::View,
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

    pub(super) fn present_network_shell(
        &mut self,
        surface: &mut GuiSurface,
        display: &mut Display,
    ) {
        self.network.present_shell(surface, display);
    }

    pub(super) fn present_imu_shell(&mut self, surface: &mut GuiSurface, display: &mut Display) {
        self.imu.present_shell(surface, display);
    }

    pub(super) fn present_microphone_shell(
        &mut self,
        surface: &mut GuiSurface,
        display: &mut Display,
    ) {
        self.microphone.present_shell(surface, display);
    }

    pub(super) fn present_camera_shell(&self, display: &mut Display) {
        self.camera.present_shell(display);
    }

    pub(super) fn present_log_shell(&mut self, surface: &mut GuiSurface, display: &mut Display) {
        self.log.present_shell(surface, display);
    }

    pub(super) fn handle_settings_pointer(
        &mut self,
        pointer: ContentPointer,
    ) -> Option<BrightnessPercent> {
        self.settings.handle_pointer(pointer)
    }

    pub(super) fn handle_speaker_pointer(
        &mut self,
        pointer: ContentPointer,
    ) -> Option<SpeakerAction> {
        self.speaker.handle_pointer(pointer)
    }

    pub(super) fn present_network(
        &mut self,
        surface: &mut GuiSurface,
        display: &mut Display,
        snapshot: &network_service::Snapshot,
    ) {
        self.network.present(surface, display, snapshot);
    }

    pub(super) fn present_imu(
        &mut self,
        surface: &mut GuiSurface,
        display: &mut Display,
        state: &ImuDisplay,
    ) {
        self.imu.present(surface, display, state);
    }

    pub(super) fn render_microphone(&self, display: &mut Display, frame: &WaveformFrame) {
        self.microphone.render_waveform(display, frame);
    }

    pub(super) fn render_camera(
        &self,
        display: &mut Display,
        frame: &mut camera_service::Frame<'_>,
    ) {
        self.camera.render(display, frame);
    }

    pub(super) fn present_speaker(
        &mut self,
        surface: &mut GuiSurface,
        display: &mut Display,
        state: SpeakerDisplay,
    ) {
        self.speaker.present(surface, display, state);
    }

    pub(super) fn present_settings(
        &mut self,
        surface: &mut GuiSurface,
        display: &mut Display,
        state: SettingsDisplay,
    ) {
        self.settings.present(surface, display, state.brightness);
    }

    pub(super) fn present_log(
        &mut self,
        surface: &mut GuiSurface,
        display: &mut Display,
        contents: &str,
    ) {
        self.log.present(surface, display, contents);
    }
}
