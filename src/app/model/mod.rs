//! CPU0 application models.
//!
//! `AppModel` is the thin application-level aggregator. Feature-specific state,
//! refresh cadence, and presentation transformations live with each application
//! as vertical migrations progress.

#[cfg(feature = "log-view")]
mod log;
mod navigation;
#[cfg(feature = "settings")]
mod settings;

use embassy_time::Instant;

#[cfg(feature = "imu-worldview")]
use crate::applications::imu_worldview;
#[cfg(feature = "mic-waveform")]
use crate::applications::mic_waveform;
#[cfg(feature = "network-demo")]
use crate::applications::network_demo;
#[cfg(feature = "speaker-synth")]
use crate::applications::speaker_synth;
#[cfg(feature = "settings")]
use crate::capabilities::display::{Brightness, BrightnessControl};
#[cfg(feature = "imu-worldview")]
use crate::capabilities::imu as imu_capability;
#[cfg(feature = "mic-waveform")]
use crate::capabilities::mic as mic_capability;
#[cfg(feature = "network-demo")]
use crate::capabilities::network as network_capability;
#[cfg(feature = "speaker-synth")]
use crate::capabilities::speaker as speaker_capability;
#[cfg(feature = "log-view")]
use crate::support::logging;

// Transitional compatibility for shell code while vertical migrations proceed.
#[cfg(feature = "imu-worldview")]
pub(crate) use crate::applications::imu_worldview::DisplayState as ImuDisplay;
#[cfg(feature = "mic-waveform")]
pub(crate) use crate::applications::mic_waveform::WaveformFrame;
pub(crate) use navigation::ViewId;
#[cfg(feature = "settings")]
pub(crate) use settings::SettingsDisplay;

pub(crate) struct AppModelInputs {
    #[cfg(feature = "network-demo")]
    pub(crate) network: network_capability::Network,
    #[cfg(feature = "imu-worldview")]
    pub(crate) imu: imu_capability::Imu,
    #[cfg(feature = "mic-waveform")]
    pub(crate) microphone: mic_capability::Microphone,
    #[cfg(feature = "speaker-synth")]
    pub(crate) speaker: speaker_capability::Speaker,
    #[cfg(feature = "settings")]
    pub(crate) brightness: BrightnessControl,
    #[cfg(feature = "log-view")]
    pub(crate) log: logging::Input,
}

pub(crate) struct AppModel {
    navigation: navigation::Model,
    #[cfg(feature = "network-demo")]
    network: network_demo::Model,
    #[cfg(feature = "imu-worldview")]
    imu: imu_worldview::Model,
    #[cfg(feature = "mic-waveform")]
    microphone: mic_waveform::Model,
    #[cfg(feature = "speaker-synth")]
    speaker: speaker_synth::Model,
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
            network: network_demo::Model::new(inputs.network),
            #[cfg(feature = "imu-worldview")]
            imu: imu_worldview::Model::new(inputs.imu),
            #[cfg(feature = "mic-waveform")]
            microphone: mic_waveform::Model::new(inputs.microphone),
            #[cfg(feature = "speaker-synth")]
            speaker: speaker_synth::Model::new(inputs.speaker),
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
        // Continuous application behaviors must keep running while another view
        // is active: speaker PCM generation and network ping/pong responses.
        #[cfg(feature = "speaker-synth")]
        self.speaker.update();
        #[cfg(feature = "network-demo")]
        self.network.update_if_due(now);

        match self.navigation.active_view() {
            #[cfg(feature = "network-demo")]
            ViewId::Network => {}
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
    pub(crate) fn apply_speaker_action(&mut self, action: speaker_synth::Action) {
        if self.navigation.active_view() == ViewId::Speaker {
            self.speaker.apply(action);
        }
    }

    #[cfg(feature = "speaker-synth")]
    pub(crate) fn take_speaker_display(&mut self) -> Option<speaker_synth::SpeakerDisplay> {
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
    pub(crate) fn take_network_display(&mut self) -> Option<network_demo::DisplayState> {
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
