//! Application navigation state.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ViewId {
    Network,
    Imu,
    Microphone,
    Speaker,
    Camera,
    Settings,
    Log,
}

impl ViewId {
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Network => "Network",
            Self::Imu => "Imu",
            Self::Microphone => "Microphone",
            Self::Speaker => "Speaker",
            Self::Camera => "Camera",
            Self::Settings => "Settings",
            Self::Log => "Log",
        }
    }
}

pub(super) struct Model {
    active_view: ViewId,
}

impl Model {
    pub(super) const fn new() -> Self {
        Self {
            active_view: ViewId::Log,
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
