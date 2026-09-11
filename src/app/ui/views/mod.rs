//! Concrete semantic view ownership.
//!
//! The top-level UI shell owns navigation and coordination. This module only
//! stores stock application view instances and their rendering/interaction state.

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
pub(crate) use speaker::Action as SpeakerAction;

pub(crate) struct Views {
    #[cfg(feature = "network-demo")]
    pub(crate) network: network::View,
    #[cfg(feature = "imu-worldview")]
    pub(crate) imu: imu::View,
    #[cfg(feature = "mic-waveform")]
    pub(crate) microphone: microphone::View,
    #[cfg(feature = "speaker-synth")]
    pub(crate) speaker: speaker::View,
    #[cfg(feature = "camera-view")]
    pub(crate) camera: camera::View,
    #[cfg(feature = "settings")]
    pub(crate) settings: settings::View,
    #[cfg(feature = "log-view")]
    pub(crate) log: log::View,
}

impl Views {
    pub(crate) fn new() -> Self {
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
