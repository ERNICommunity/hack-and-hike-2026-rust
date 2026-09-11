//! CPU0 application models.
//!
//! `AppModel` is the thin application-level aggregator. Feature-specific state,
//! refresh cadence, and presentation transformations live with each application
//! as vertical migrations progress.

#[cfg(feature = "log-view")]
mod log;
#[cfg(feature = "mic-waveform")]
mod microphone;
mod navigation;
#[cfg(feature = "network-demo")]
mod network;
#[cfg(feature = "settings")]
mod settings;
#[cfg(feature = "speaker-synth")]
mod speaker;

use embassy_time::Instant;

#[cfg(feature = "imu-worldview")]
use crate::applications::imu_worldview;
#[cfg(any(feature = "mic-waveform", feature = "speaker-synth"))]
use crate::capabilities::audio;
#[cfg(feature = "settings")]
use crate::capabilities::display::{Brightness, BrightnessControl};
#[cfg(feature = "imu-worldview")]
use crate::capabilities::imu as imu_capability;
#[cfg(feature = "network-demo")]
use crate::capabilities::network as network_capability;
#[cfg(feature = "log-view")]
use crate::support::logging;

// Transitional compatibility for renderer helpers that are being migrated out
// of the horizontal app layer one application at a time.
#[cfg(feature = "imu-worldview")]
pub(crate) use crate::applications::imu_worldview::DisplayState as ImuDisplay;
#[cfg(feature = "mic-waveform")]
pub(crate) use microphone::{MAX_AMPLITUDE_PIXELS, POINTS, WaveformFrame};
pub(crate) use navigation::ViewId;
#[cfg(feature = "settings")]
pub(crate) use settings::SettingsDisplay;
#[cfg(feature = "speaker-synth")]
pub(crate) use speaker::SpeakerDisplay;

pub(crate) struct AppModelInputs {
    #[cfg(feature = "network-demo")]
    pub(crate) network: network_capability::Input,
    #[cfg(feature = "imu-worldview")]
    pub(crate) imu: imu_capability::Imu,
    #[cfg(feature = "mic-waveform")]
    pub(crate) audio: audio::Input,
    #[cfg(feature = "speaker-synth")]
    pub(crate) playback: audio::PlaybackControl,
    #[cfg(feature = "settings")]
    pub(crate) brightness: BrightnessControl,
    #[cfg(feature = "log-view")]
    pub(crate) log: logging::Input,
}

pub(crate) struct AppModel {
    navigation: navigation::Model,
    #[cfg(feature = "network-demo")]
    network: network::Model,
    #[cfg(feature = "imu-worldview")]
    imu: imu_worldview::Model,
    #[cfg(feature = "mic-waveform")]
    microphone: microphone::Model,
    #[cfg(feature = "speaker-synth")]
    speaker: speaker::Model,
    #[cfg(feature = "settings")]
    settings: settings::Model,
    #[cfg(feature = "log-view")]
    log: log::Model,
}

impl AppModel {
    pub(crate) fn new(inputs: AppModelInputs) -> Self {
        Self {
            navigation: navigation::Model::new(),
            #[cfg(feature = "network-demo")]
            network: network::Model::new(inputs.network),
            #[cfg(feature = "imu-worldview")]
            imu: imu_worldview::Model::new(inputs.imu),
            #[cfg(feature = "mic-waveform")]
            microphone: microphone::Model::new(inputs.audio),
            #[cfg(feature = "speaker-synth")]
            speaker: speaker::Model::new(inputs.playback),
            #[cfg(feature = "settings")]
            settings: settings::Model::new(inputs.brightness),
            #[cfg(feature = "log-view")]
            log: {
                let mut model = log::Model::new(inputs.log);
                model.refresh();
                model
            },
        }
    }

    pub(crate) fn active_view(&self) -> ViewId {
        self.navigation.active_view()
    }

    pub(crate) fn request_view(&mut self, view: ViewId) {
        if !self.navigation.request_view(view) {
            return;
        }

        match view {
            #[cfg(feature = "network-demo")]
            ViewId::Network => self.network.mark_dirty(),
            #[cfg(feature = "imu-worldview")]
            ViewId::Imu => self.imu.mark_dirty(),
            #[cfg(feature = "mic-waveform")]
            ViewId::Microphone => self.microphone.mark_dirty(),
            #[cfg(feature = "speaker-synth")]
            ViewId::Speaker => self.speaker.mark_dirty(),
            #[cfg(feature = "camera-view")]
            ViewId::Camera => {}
            #[cfg(feature = "settings")]
            ViewId::Settings => self.settings.mark_dirty(),
            #[cfg(feature = "log-view")]
            ViewId::Log => self.log.mark_dirty(),
        }
    }

    pub(crate) fn update(&mut self, now: Instant) {
        match self.navigation.active_view() {
            #[cfg(feature = "network-demo")]
            ViewId::Network => self.network.update_if_due(now),
            #[cfg(feature = "imu-worldview")]
            ViewId::Imu => self.imu.update_if_due(now),
            #[cfg(feature = "mic-waveform")]
            ViewId::Microphone => self.microphone.update_if_due(now),
            #[cfg(feature = "log-view")]
            ViewId::Log => self.log.update_if_due(now),
            #[cfg(feature = "camera-view")]
            ViewId::Camera => {}
            #[cfg(feature = "settings")]
            ViewId::Settings => {}
            #[cfg(feature = "speaker-synth")]
            ViewId::Speaker => {}
        }
    }

    #[cfg(feature = "settings")]
    pub(crate) fn set_brightness(&mut self, brightness: Brightness) {
        if self.navigation.active_view() == ViewId::Settings {
            self.settings.set_brightness(brightness);
        }
    }

    #[cfg(feature = "speaker-synth")]
    pub(crate) fn toggle_speaker_playback(&mut self) {
        if self.navigation.active_view() == ViewId::Speaker {
            self.speaker.toggle_playback();
        }
    }

    #[cfg(feature = "speaker-synth")]
    pub(crate) fn play_speaker_one_shot(&mut self) {
        if self.navigation.active_view() == ViewId::Speaker {
            self.speaker.play_one_shot();
        }
    }

    #[cfg(feature = "speaker-synth")]
    pub(crate) fn set_speaker_tempo(&mut self, tempo: audio::TempoBpm) {
        if self.navigation.active_view() == ViewId::Speaker {
            self.speaker.set_tempo(tempo);
        }
    }

    #[cfg(feature = "speaker-synth")]
    pub(crate) fn set_speaker_pitch(&mut self, pitch: audio::PitchSemitones) {
        if self.navigation.active_view() == ViewId::Speaker {
            self.speaker.set_pitch(pitch);
        }
    }

    #[cfg(feature = "speaker-synth")]
    pub(crate) fn take_speaker_display(&mut self) -> Option<SpeakerDisplay> {
        if self.navigation.active_view() != ViewId::Speaker {
            return None;
        }
        self.speaker.take_display()
    }

    #[cfg(feature = "settings")]
    pub(crate) fn take_settings_display(&mut self) -> Option<SettingsDisplay> {
        if self.navigation.active_view() != ViewId::Settings {
            return None;
        }
        self.settings.take_display()
    }

    #[cfg(feature = "network-demo")]
    pub(crate) fn take_network_display(&mut self) -> Option<network_capability::Snapshot> {
        if self.navigation.active_view() != ViewId::Network {
            return None;
        }
        self.network.take_display()
    }

    #[cfg(feature = "log-view")]
    pub(crate) fn with_log_text<R>(&mut self, render: impl FnOnce(&str) -> R) -> Option<R> {
        if self.navigation.active_view() != ViewId::Log {
            return None;
        }
        self.log.with_text(render)
    }

    #[cfg(feature = "imu-worldview")]
    pub(crate) fn take_imu_display(&mut self) -> Option<ImuDisplay> {
        if self.navigation.active_view() != ViewId::Imu {
            return None;
        }
        self.imu.take_display()
    }

    #[cfg(feature = "mic-waveform")]
    pub(crate) fn take_waveform_frame(&mut self) -> Option<WaveformFrame> {
        if self.navigation.active_view() != ViewId::Microphone {
            return None;
        }
        self.microphone.take_frame()
    }
}
