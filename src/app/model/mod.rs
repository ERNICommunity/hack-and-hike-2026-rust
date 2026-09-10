//! CPU0 application models.
//!
//! `AppModel` is the thin application-level aggregator. Feature-specific state,
//! refresh cadence, and presentation transformations live in sibling modules so
//! each model can be understood independently of the other screens.

mod imu;
mod log;
pub(crate) mod microphone;
mod navigation;
mod network;
mod settings;
mod speaker;

use embassy_time::Instant;

use crate::{
    audio,
    display_control::{BrightnessControl, BrightnessPercent},
    imu as imu_service, logger,
    network as network_service,
};

pub(crate) use imu::ImuDisplay;
pub(crate) use microphone::WaveformFrame;
pub(crate) use navigation::ViewId;
pub(crate) use settings::SettingsDisplay;
pub(crate) use speaker::SpeakerDisplay;

pub struct AppModelInputs {
    pub network: network_service::Input,
    pub imu: imu_service::Input,
    pub audio: audio::Input,
    pub log: logger::Input,
}

pub struct AppModel {
    navigation: navigation::Model,
    network: network::Model,
    imu: imu::Model,
    microphone: microphone::Model,
    speaker: speaker::Model,
    settings: settings::Model,
    log: log::Model,
}

impl AppModel {
    pub fn new(
        inputs: AppModelInputs,
        brightness: BrightnessControl,
        playback: audio::PlaybackControl,
    ) -> Self {
        let AppModelInputs {
            network,
            imu,
            audio,
            log,
        } = inputs;
        let mut log = log::Model::new(log);
        log.refresh();

        Self {
            navigation: navigation::Model::new(),
            network: network::Model::new(network),
            imu: imu::Model::new(imu),
            microphone: microphone::Model::new(audio),
            speaker: speaker::Model::new(playback),
            settings: settings::Model::new(brightness),
            log,
        }
    }

    pub fn active_view(&self) -> ViewId {
        self.navigation.active_view()
    }

    pub fn request_view(&mut self, view: ViewId) {
        if !self.navigation.request_view(view) {
            return;
        }

        match view {
            ViewId::Network => self.network.mark_dirty(),
            ViewId::Imu => self.imu.mark_dirty(),
            ViewId::Microphone => self.microphone.mark_dirty(),
            ViewId::Speaker => self.speaker.mark_dirty(),
            ViewId::Camera => {}
            ViewId::Settings => self.settings.mark_dirty(),
            ViewId::Log => self.log.mark_dirty(),
        }
    }

    pub fn update(&mut self, now: Instant) {
        match self.navigation.active_view() {
            ViewId::Network => self.network.update_if_due(now),
            ViewId::Imu => self.imu.update_if_due(now),
            ViewId::Microphone => self.microphone.update_if_due(now),
            ViewId::Log => self.log.update_if_due(now),
            ViewId::Camera | ViewId::Settings | ViewId::Speaker => {}
        }
    }

    pub fn set_brightness(&mut self, brightness: BrightnessPercent) {
        if self.navigation.active_view() == ViewId::Settings {
            self.settings.set_brightness(brightness);
        }
    }

    pub fn toggle_speaker_playback(&mut self) {
        if self.navigation.active_view() == ViewId::Speaker {
            self.speaker.toggle_playback();
        }
    }

    pub fn play_speaker_one_shot(&mut self) {
        if self.navigation.active_view() == ViewId::Speaker {
            self.speaker.play_one_shot();
        }
    }

    pub fn set_speaker_tempo(&mut self, tempo: audio::TempoBpm) {
        if self.navigation.active_view() == ViewId::Speaker {
            self.speaker.set_tempo(tempo);
        }
    }

    pub fn set_speaker_pitch(&mut self, pitch: audio::PitchSemitones) {
        if self.navigation.active_view() == ViewId::Speaker {
            self.speaker.set_pitch(pitch);
        }
    }

    pub fn take_speaker_display(&mut self) -> Option<SpeakerDisplay> {
        if self.navigation.active_view() != ViewId::Speaker {
            return None;
        }
        self.speaker.take_display()
    }

    pub fn take_settings_display(&mut self) -> Option<SettingsDisplay> {
        if self.navigation.active_view() != ViewId::Settings {
            return None;
        }
        self.settings.take_display()
    }

    pub fn take_network_display(&mut self) -> Option<network_service::Snapshot> {
        if self.navigation.active_view() != ViewId::Network {
            return None;
        }
        self.network.take_display()
    }

    pub fn with_log_text<R>(&mut self, render: impl FnOnce(&str) -> R) -> Option<R> {
        if self.navigation.active_view() != ViewId::Log {
            return None;
        }
        self.log.with_text(render)
    }

    pub fn take_imu_display(&mut self) -> Option<ImuDisplay> {
        if self.navigation.active_view() != ViewId::Imu {
            return None;
        }
        self.imu.take_display()
    }

    pub fn take_waveform_frame(&mut self) -> Option<WaveformFrame> {
        if self.navigation.active_view() != ViewId::Microphone {
            return None;
        }
        self.microphone.take_frame()
    }
}
