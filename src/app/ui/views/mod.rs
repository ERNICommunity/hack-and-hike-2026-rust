//! Semantic view ownership and dispatch.
//!
//! Every screen owns a KDL-backed fixed-capacity context in its matching module.
//! KDL is the single source of normal page geometry. View-specific overlays stay
//! private to their semantic view; only genuinely reusable drawing primitives
//! live in `common`.

mod camera;
mod common;
mod imu;
mod log;
mod microphone;
mod network;
mod settings;
mod speaker;

use crate::{
    app::model::{ImuDisplay, SettingsDisplay, SpeakerDisplay, ViewId, WaveformFrame},
    services::{
        audio::{PitchSemitones, TempoBpm},
        camera as camera_service,
        display::{BrightnessPercent, Display},
        network as network_service,
    },
};

use super::{gui::GuiSurface, navigation::ContentPointer};

pub(super) enum Interaction {
    SetBrightness(BrightnessPercent),
    ToggleSpeakerPlayback,
    PlaySpeakerOneShot,
    SetSpeakerTempo(TempoBpm),
    SetSpeakerPitch(PitchSemitones),
}

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

    pub(super) fn present_shell(
        &mut self,
        view: ViewId,
        surface: &mut GuiSurface,
        display: &mut Display,
        settings_display: Option<SettingsDisplay>,
        speaker_display: Option<SpeakerDisplay>,
    ) {
        match view {
            ViewId::Network => self.network.present_shell(surface, display),
            ViewId::Imu => self.imu.present_shell(surface, display),
            ViewId::Microphone => self.microphone.present_shell(surface, display),
            ViewId::Speaker => self.speaker.present(
                surface,
                display,
                speaker_display.expect("speaker state must be available when presenting Speaker"),
            ),
            ViewId::Camera => self.camera.present_shell(display),
            ViewId::Settings => self.settings.present(
                surface,
                display,
                settings_display.expect("settings state must be available when presenting Settings"),
            ),
            ViewId::Log => self.log.present_shell(surface, display),
        }
    }

    pub(super) fn handle_pointer(
        &mut self,
        view: ViewId,
        pointer: ContentPointer,
    ) -> Option<Interaction> {
        match view {
            ViewId::Settings => self
                .settings
                .handle_pointer(pointer)
                .map(Interaction::SetBrightness),
            ViewId::Speaker => self.speaker.handle_pointer(pointer).map(|action| match action {
                speaker::Action::TogglePlayback => Interaction::ToggleSpeakerPlayback,
                speaker::Action::PlayOneShot => Interaction::PlaySpeakerOneShot,
                speaker::Action::SetTempo(tempo) => Interaction::SetSpeakerTempo(tempo),
                speaker::Action::SetPitch(pitch) => Interaction::SetSpeakerPitch(pitch),
            }),
            _ => None,
        }
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
