//! Firmware application shell.
//!
//! The shell owns navigation state/chrome, touch routing, and the one reusable
//! PSRAM embedded-gui framebuffer. Concrete applications own all application
//! behavior, state, and Views. Dispatch stays explicit; there is no common
//! Application or View trait.

pub(crate) mod common;
pub(crate) mod design;
pub(crate) mod gui;
pub(crate) mod navigation;
pub(crate) mod theme;

use embassy_time::Instant;

#[cfg(feature = "camera-view")]
use crate::capabilities::camera;
#[cfg(feature = "touch")]
use crate::capabilities::touch;
use crate::{
    applications::Applications,
    capabilities::display::Display,
};

use gui::GuiSurface;
use navigation::NavigationInput;
pub(crate) use navigation::ViewId;

#[derive(Clone, Copy, Debug)]
pub(crate) struct ViewTransition {
    pub(crate) from: ViewId,
    pub(crate) to: ViewId,
}

pub(crate) struct Ui {
    applications: Applications,
    navigation: NavigationInput,
    gui_surface: GuiSurface,
    active_view: ViewId,
    presented_view: ViewId,
}

impl Ui {
    #[cfg(feature = "touch")]
    pub(crate) fn new(applications: Applications, touch: touch::Touch) -> Self {
        Self::with_navigation(applications, NavigationInput::new(touch))
    }

    #[cfg(not(feature = "touch"))]
    pub(crate) fn new(applications: Applications) -> Self {
        Self::with_navigation(applications, NavigationInput::new())
    }

    fn with_navigation(applications: Applications, navigation: NavigationInput) -> Self {
        let active_view = ViewId::initial();
        Self {
            applications,
            navigation,
            gui_surface: GuiSurface::new(),
            active_view,
            presented_view: active_view,
        }
    }

    pub(crate) fn presented_view(&self) -> ViewId {
        self.presented_view
    }

    pub(crate) fn render_initial(&mut self, display: &mut Display) {
        {
            let mut navigation_surface = display.surface(design::NAV_REGION);
            navigation::render(&mut navigation_surface, self.presented_view);
        }
        self.present_current_view(display);
    }

    pub(crate) fn prepare_frame(&mut self, now: Instant) -> Option<ViewTransition> {
        let active_view = self.active_view;
        let selected = {
            let navigation = &mut self.navigation;
            let applications = &mut self.applications;
            navigation.poll(|pointer| match active_view {
                #[cfg(feature = "settings")]
                ViewId::Settings => applications.settings.handle_pointer(pointer),
                #[cfg(feature = "speaker-synth")]
                ViewId::Speaker => applications.speaker_synth.handle_pointer(pointer),
                _ => {}
            })
        };

        if let Some(view) = selected {
            if view != self.active_view {
                self.active_view = view;
                self.mark_active_dirty();
            }
        }

        // Continuous application behaviors keep running while another view is
        // active, matching the existing speaker and network semantics.
        #[cfg(feature = "speaker-synth")]
        self.applications.speaker_synth.update();
        #[cfg(feature = "network-demo")]
        self.applications.network_demo.update_if_due(now);

        match self.active_view {
            #[cfg(feature = "network-demo")]
            ViewId::Network => {}
            #[cfg(feature = "imu-worldview")]
            ViewId::Imu => self.applications.imu_worldview.update_if_due(now),
            #[cfg(feature = "mic-waveform")]
            ViewId::Microphone => self.applications.mic_waveform.update_if_due(now),
            #[cfg(feature = "speaker-synth")]
            ViewId::Speaker => {}
            #[cfg(feature = "camera-view")]
            ViewId::Camera => {}
            #[cfg(feature = "settings")]
            ViewId::Settings => {}
            #[cfg(feature = "log-view")]
            ViewId::Log => self.applications.log_view.update_if_due(now),
        }

        (self.active_view != self.presented_view).then_some(ViewTransition {
            from: self.presented_view,
            to: self.active_view,
        })
    }

    fn mark_active_dirty(&mut self) {
        match self.active_view {
            #[cfg(feature = "network-demo")]
            ViewId::Network => self.applications.network_demo.mark_dirty(),
            #[cfg(feature = "imu-worldview")]
            ViewId::Imu => self.applications.imu_worldview.mark_dirty(),
            #[cfg(feature = "mic-waveform")]
            ViewId::Microphone => self.applications.mic_waveform.mark_dirty(),
            #[cfg(feature = "speaker-synth")]
            ViewId::Speaker => self.applications.speaker_synth.mark_dirty(),
            #[cfg(feature = "camera-view")]
            ViewId::Camera => {}
            #[cfg(feature = "settings")]
            ViewId::Settings => self.applications.settings.mark_dirty(),
            #[cfg(feature = "log-view")]
            ViewId::Log => self.applications.log_view.mark_dirty(),
        }
    }

    pub(crate) fn apply_navigation(&mut self, transition: ViewTransition, display: &mut Display) {
        debug_assert_eq!(transition.from, self.presented_view);
        debug_assert_eq!(transition.to, self.active_view);
        self.presented_view = transition.to;
        {
            let mut navigation_surface = display.surface(design::NAV_REGION);
            navigation::render(&mut navigation_surface, transition.to);
        }
        self.present_current_view(display);
    }

    pub(crate) fn render(&mut self, display: &mut Display) {
        let mut content = display.surface(design::CONTENT_REGION);
        match self.presented_view {
            #[cfg(feature = "network-demo")]
            ViewId::Network => {
                let _ = self
                    .applications
                    .network_demo
                    .present_if_dirty(&mut self.gui_surface, &mut content);
            }
            #[cfg(feature = "imu-worldview")]
            ViewId::Imu => {
                let _ = self
                    .applications
                    .imu_worldview
                    .present_if_dirty(&mut self.gui_surface, &mut content);
            }
            #[cfg(feature = "mic-waveform")]
            ViewId::Microphone => {
                let _ = self.applications.mic_waveform.render_if_dirty(&mut content);
            }
            #[cfg(feature = "speaker-synth")]
            ViewId::Speaker => {
                let _ = self
                    .applications
                    .speaker_synth
                    .present_if_dirty(&mut self.gui_surface, &mut content);
            }
            #[cfg(feature = "camera-view")]
            ViewId::Camera => {}
            #[cfg(feature = "settings")]
            ViewId::Settings => {
                let _ = self
                    .applications
                    .settings
                    .present_if_dirty(&mut self.gui_surface, &mut content);
            }
            #[cfg(feature = "log-view")]
            ViewId::Log => {
                let _ = self
                    .applications
                    .log_view
                    .present_if_dirty(&mut self.gui_surface, &mut content);
            }
        }
    }

    #[cfg(feature = "camera-view")]
    pub(crate) fn render_camera(&self, display: &mut Display, frame: &mut camera::Frame<'_>) {
        if self.presented_view == ViewId::Camera {
            let mut content = display.surface(design::CONTENT_REGION);
            self.applications.camera_view.render(&mut content, frame);
        }
    }

    fn present_current_view(&mut self, display: &mut Display) {
        let mut content = display.surface(design::CONTENT_REGION);
        match self.presented_view {
            #[cfg(feature = "network-demo")]
            ViewId::Network => self
                .applications
                .network_demo
                .present_shell(&mut self.gui_surface, &mut content),
            #[cfg(feature = "imu-worldview")]
            ViewId::Imu => self
                .applications
                .imu_worldview
                .present_shell(&mut self.gui_surface, &mut content),
            #[cfg(feature = "mic-waveform")]
            ViewId::Microphone => self
                .applications
                .mic_waveform
                .present_shell(&mut self.gui_surface, &mut content),
            #[cfg(feature = "speaker-synth")]
            ViewId::Speaker => {
                let presented = self
                    .applications
                    .speaker_synth
                    .present_if_dirty(&mut self.gui_surface, &mut content);
                debug_assert!(presented, "speaker state must be dirty when entering Speaker");
            }
            #[cfg(feature = "camera-view")]
            ViewId::Camera => self.applications.camera_view.present_shell(&mut content),
            #[cfg(feature = "settings")]
            ViewId::Settings => {
                let presented = self
                    .applications
                    .settings
                    .present_if_dirty(&mut self.gui_surface, &mut content);
                debug_assert!(presented, "settings state must be dirty when entering Settings");
            }
            #[cfg(feature = "log-view")]
            ViewId::Log => self
                .applications
                .log_view
                .present_current(&mut self.gui_surface, &mut content),
        }
    }
}
