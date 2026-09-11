//! Stock applications built from semantic hardware capabilities.
//!
//! Each vertical application owns its behavior, application state, and concrete
//! view. `Applications` is only the concrete feature-composed host; the UI shell
//! performs direct explicit dispatch and no common application/view trait exists.

#[cfg(feature = "camera-view")]
pub(crate) mod camera_view;
#[cfg(feature = "imu-worldview")]
pub(crate) mod imu_worldview;
#[cfg(feature = "log-view")]
pub(crate) mod log_view;
#[cfg(feature = "mic-waveform")]
pub(crate) mod mic_waveform;
#[cfg(feature = "network-demo")]
pub(crate) mod network_demo;
#[cfg(feature = "settings")]
pub(crate) mod settings;
#[cfg(feature = "speaker-synth")]
pub(crate) mod speaker_synth;

pub(crate) struct Inputs {
    #[cfg(feature = "network-demo")]
    pub(crate) network: crate::capabilities::network::Network,
    #[cfg(feature = "imu-worldview")]
    pub(crate) imu: crate::capabilities::imu::Imu,
    #[cfg(feature = "mic-waveform")]
    pub(crate) microphone: crate::capabilities::mic::Microphone,
    #[cfg(feature = "speaker-synth")]
    pub(crate) speaker: crate::capabilities::speaker::Speaker,
    #[cfg(feature = "settings")]
    pub(crate) brightness: crate::capabilities::display::BrightnessControl,
    #[cfg(feature = "log-view")]
    pub(crate) log: crate::support::logging::Input,
}

pub(crate) struct Applications {
    #[cfg(feature = "network-demo")]
    pub(crate) network_demo: network_demo::Application,
    #[cfg(feature = "imu-worldview")]
    pub(crate) imu_worldview: imu_worldview::Application,
    #[cfg(feature = "mic-waveform")]
    pub(crate) mic_waveform: mic_waveform::Application,
    #[cfg(feature = "speaker-synth")]
    pub(crate) speaker_synth: speaker_synth::Application,
    #[cfg(feature = "camera-view")]
    pub(crate) camera_view: camera_view::Application,
    #[cfg(feature = "settings")]
    pub(crate) settings: settings::Application,
    #[cfg(feature = "log-view")]
    pub(crate) log_view: log_view::Application,
}

impl Applications {
    pub(crate) fn new(inputs: Inputs) -> Self {
        Self {
            #[cfg(feature = "network-demo")]
            network_demo: network_demo::Application::new(inputs.network),
            #[cfg(feature = "imu-worldview")]
            imu_worldview: imu_worldview::Application::new(inputs.imu),
            #[cfg(feature = "mic-waveform")]
            mic_waveform: mic_waveform::Application::new(inputs.microphone),
            #[cfg(feature = "speaker-synth")]
            speaker_synth: speaker_synth::Application::new(inputs.speaker),
            #[cfg(feature = "camera-view")]
            camera_view: camera_view::Application::new(),
            #[cfg(feature = "settings")]
            settings: settings::Application::new(inputs.brightness),
            #[cfg(feature = "log-view")]
            log_view: log_view::Application::new(inputs.log),
        }
    }
}
