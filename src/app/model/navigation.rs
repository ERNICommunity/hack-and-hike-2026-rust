//! Application navigation state.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ViewId {
    #[cfg(feature = "network-demo")]
    Network,
    #[cfg(feature = "imu-worldview")]
    Imu,
    #[cfg(feature = "mic-waveform")]
    Microphone,
    #[cfg(feature = "speaker-synth")]
    Speaker,
    #[cfg(feature = "camera-view")]
    Camera,
    #[cfg(feature = "settings")]
    Settings,
    #[cfg(feature = "log-view")]
    Log,
}

impl ViewId {
    pub(crate) const fn name(self) -> &'static str {
        match self {
            #[cfg(feature = "network-demo")]
            Self::Network => "Network",
            #[cfg(feature = "imu-worldview")]
            Self::Imu => "Imu",
            #[cfg(feature = "mic-waveform")]
            Self::Microphone => "Microphone",
            #[cfg(feature = "speaker-synth")]
            Self::Speaker => "Speaker",
            #[cfg(feature = "camera-view")]
            Self::Camera => "Camera",
            #[cfg(feature = "settings")]
            Self::Settings => "Settings",
            #[cfg(feature = "log-view")]
            Self::Log => "Log",
        }
    }
}

pub(super) const ENABLED_VIEWS: &[ViewId] = &[
    #[cfg(feature = "network-demo")]
    ViewId::Network,
    #[cfg(feature = "imu-worldview")]
    ViewId::Imu,
    #[cfg(feature = "mic-waveform")]
    ViewId::Microphone,
    #[cfg(feature = "speaker-synth")]
    ViewId::Speaker,
    #[cfg(feature = "camera-view")]
    ViewId::Camera,
    #[cfg(feature = "settings")]
    ViewId::Settings,
    #[cfg(feature = "log-view")]
    ViewId::Log,
];

pub(super) struct Model {
    active_view: ViewId,
}

impl Model {
    pub(super) const fn new() -> Self {
        #[cfg(feature = "log-view")]
        let active_view = ViewId::Log;
        #[cfg(not(feature = "log-view"))]
        let active_view = ENABLED_VIEWS[0];

        Self { active_view }
    }

    pub(super) const fn active_view(&self) -> ViewId {
        self.active_view
    }

    /// Apply a requested view and report whether navigation actually changed.
    pub(super) fn request_view(&mut self, view: ViewId) -> bool {
        if view == self.active_view {
            return false;
        }
        self.active_view = view;
        true
    }
}
