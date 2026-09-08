//! Semantic view ownership and dispatch.
//!
//! Every screen owns a KDL-backed fixed-capacity context in its matching module.
//! KDL is the single source of normal page geometry. View-specific overlays stay
//! private to their semantic view; only genuinely reusable drawing primitives
//! live in `common`.

mod common;
mod imu;
mod log;
mod microphone;
mod network;
mod settings;
mod speaker;

use crate::{
    audio::{PitchSemitones, TempoBpm},
    display::Display,
    display_control::BrightnessPercent,
    models::{ImuDisplay, SettingsDisplay, SpeakerDisplay, ViewId},
    network as network_service, theme,
    waveform::WaveformFrame,
};

use super::{design, gui::GuiSurface, navigation::ContentPointer};

const CAMERA_SCALE: usize = 8;

pub(crate) enum Interaction {
    SetBrightness(BrightnessPercent),
    ToggleSpeakerPlayback,
    PlaySpeakerOneShot,
    SetSpeakerTempo(TempoBpm),
    SetSpeakerPitch(PitchSemitones),
}

pub(crate) struct Views {
    network: network::View,
    imu: imu::View,
    microphone: microphone::View,
    speaker: speaker::View,
    settings: settings::View,
    log: log::View,
}

impl Views {
    pub(crate) fn new() -> Self {
        Self {
            network: network::View::new(),
            imu: imu::View::new(),
            microphone: microphone::View::new(),
            speaker: speaker::View::new(),
            settings: settings::View::new(BrightnessPercent::FULL),
            log: log::View::new(),
        }
    }

    pub(crate) fn present_shell(
        &mut self,
        view: ViewId,
        surface: &mut GuiSurface,
        display: &mut Display,
        settings_display: Option<SettingsDisplay>,
    ) {
        match view {
            ViewId::Network => self.network.present_shell(surface, display),
            ViewId::Imu => self.imu.present_shell(surface, display),
            ViewId::Microphone => self.microphone.present_shell(surface, display),
            ViewId::Speaker => self.speaker.present(surface, display),
            ViewId::Camera => present_camera(display),
            ViewId::Settings => {
                if let Some(state) = settings_display {
                    self.settings.sync_brightness(state.brightness);
                }
                self.settings.present(surface, display);
            }
            ViewId::Log => self.log.present_shell(surface, display),
        }
    }

    pub(crate) fn handle_pointer(
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

    pub(crate) fn present_network(
        &mut self,
        surface: &mut GuiSurface,
        display: &mut Display,
        snapshot: &network_service::Snapshot,
    ) {
        self.network.present(surface, display, snapshot);
    }

    pub(crate) fn present_imu(
        &mut self,
        surface: &mut GuiSurface,
        display: &mut Display,
        state: &ImuDisplay,
    ) {
        self.imu.present(surface, display, state);
    }

    pub(crate) fn render_microphone(&self, display: &mut Display, frame: &WaveformFrame) {
        self.microphone.render_waveform(display, frame);
    }

    pub(crate) fn present_speaker(
        &mut self,
        surface: &mut GuiSurface,
        display: &mut Display,
        state: SpeakerDisplay,
    ) {
        self.speaker.sync(state);
        self.speaker.present(surface, display);
    }

    pub(crate) fn present_settings(
        &mut self,
        surface: &mut GuiSurface,
        display: &mut Display,
        state: SettingsDisplay,
    ) {
        self.settings.sync_brightness(state.brightness);
        self.settings.present(surface, display);
    }

    pub(crate) fn present_log(
        &mut self,
        surface: &mut GuiSurface,
        display: &mut Display,
        contents: &str,
    ) {
        self.log.present(surface, display, contents);
    }
}

fn present_camera(display: &mut Display) {
    let image_width = design::NAV_ICON_SIZE * CAMERA_SCALE;
    let image_height = design::NAV_ICON_SIZE * CAMERA_SCALE;
    let image_x = (design::CONTENT_WIDTH - image_width) / 2;
    let image_y = (design::CONTENT_HEIGHT - image_height) / 2;

    display.render_scanlines(design::CONTENT_REGION, |local_y, pixels| {
        pixels.fill(theme::WHITE_RGB565);

        if !(image_y..image_y + image_height).contains(&local_y) {
            return;
        }

        let icon_y = (local_y - image_y) / CAMERA_SCALE;
        let row_bits = design::CAMERA_ICON[icon_y];
        for icon_x in 0..design::NAV_ICON_SIZE {
            if row_bits & (1 << (design::NAV_ICON_SIZE - 1 - icon_x)) == 0 {
                continue;
            }

            let start = image_x + icon_x * CAMERA_SCALE;
            pixels[start..start + CAMERA_SCALE].fill(theme::DARK_BLUE_RGB565);
        }
    });
}
