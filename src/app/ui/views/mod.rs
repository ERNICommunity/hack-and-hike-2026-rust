//! Concrete semantic view ownership.
//!
//! `Ui` owns navigation and coordination. This module only stores the concrete
//! view instances; each view keeps its own rendering and interaction mechanics.

#[cfg(feature = "camera-view")]
mod camera;
mod common;
#[cfg(feature = "imu-worldview")]
mod imu;
#[cfg(feature = "log-view")]
mod log;
#[cfg(feature = "mic-waveform")]
mod microphone;
#[cfg(feature = "network-demo")]
mod network;
#[cfg(feature = "settings")]
mod settings;
#[cfg(feature = "speaker-synth")]
mod speaker;

#[cfg(feature = "speaker-synth")]
pub(super) use speaker::Action as SpeakerAction;

pub(super) struct Views {
    #[cfg(feature = "network-demo")]
    pub(super) network: network::View,
    #[cfg(feature = "imu-worldview")]
    pub(super) imu: imu::View,
    #[cfg(feature = "mic-waveform")]
    pub(super) microphone: microphone::View,
    #[cfg(feature = "speaker-synth")]
    pub(super) speaker: speaker::View,
    #[cfg(feature = "camera-view")]
    pub(super) camera: camera::View,
    #[cfg(feature = "settings")]
    pub(super) settings: settings::View,
    #[cfg(feature = "log-view")]
    pub(super) log: log::View,
}

impl Views {
    pub(super) fn new() -> Self {
        Self {
            #[cfg(feature = "network-demo")]
            network: network::View::new(),
            #[cfg(feature = "imu-worldview")]
            imu: imu::View::new(),
            #[cfg(feature = "mic-waveform")]
            microphone: microphone::View::new(),
            #[cfg(feature = "speaker-synth")]
            speaker: speaker::View::new(),
            #[cfg(feature = "camera-view")]
            camera: camera::View::new(),
            #[cfg(feature = "settings")]
            settings: settings::View::new(),
            #[cfg(feature = "log-view")]
            log: log::View::new(),
        }
    }
}
