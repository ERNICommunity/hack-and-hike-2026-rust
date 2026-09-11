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

    const fn initial() -> Self {
        // Preserve the existing full/default behavior: Log is the preferred
        // initial view whenever it is enabled. Without Log, select the first
        // enabled application in navigation order.
        #[cfg(feature = "log-view")]
        {
            Self::Log
        }
        #[cfg(all(not(feature = "log-view"), feature = "network-demo"))]
        {
            Self::Network
        }
        #[cfg(all(
            not(feature = "log-view"),
            not(feature = "network-demo"),
            feature = "imu-worldview"
        ))]
        {
            Self::Imu
        }
        #[cfg(all(
            not(feature = "log-view"),
            not(feature = "network-demo"),
            not(feature = "imu-worldview"),
            feature = "mic-waveform"
        ))]
        {
            Self::Microphone
        }
        #[cfg(all(
            not(feature = "log-view"),
            not(feature = "network-demo"),
            not(feature = "imu-worldview"),
            not(feature = "mic-waveform"),
            feature = "speaker-synth"
        ))]
        {
            Self::Speaker
        }
        #[cfg(all(
            not(feature = "log-view"),
            not(feature = "network-demo"),
            not(feature = "imu-worldview"),
            not(feature = "mic-waveform"),
            not(feature = "speaker-synth"),
            feature = "camera-view"
        ))]
        {
            Self::Camera
        }
        #[cfg(all(
            not(feature = "log-view"),
            not(feature = "network-demo"),
            not(feature = "imu-worldview"),
            not(feature = "mic-waveform"),
            not(feature = "speaker-synth"),
            not(feature = "camera-view"),
            feature = "settings"
        ))]
        {
            Self::Settings
        }
    }
}

pub(super) struct Model {
    active_view: ViewId,
}

impl Model {
    pub(super) const fn new() -> Self {
        Self {
            active_view: ViewId::initial(),
        }
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
