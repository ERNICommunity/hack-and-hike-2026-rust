//! Firmware application shell.
//!
//! The shell owns navigation chrome, touch routing, and the one reusable PSRAM
//! embedded-gui framebuffer. Stock applications receive only the bounded content
//! `Surface`; the navigation rail is rendered through a separate shell-owned
//! surface and raw `Display` access never crosses into application views.

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
    app::{model::{AppModel, ViewId}, ui::views::Views},
    capabilities::display::Display,
};

use gui::GuiSurface;
use navigation::NavigationInput;

#[derive(Clone, Copy, Debug)]
pub(crate) struct ViewTransition {
    pub(crate) from: ViewId,
    pub(crate) to: ViewId,
}

pub(crate) struct Ui {
    model: AppModel,
    navigation: NavigationInput,
    views: Views,
    gui_surface: GuiSurface,
    presented_view: ViewId,
}

impl Ui {
    #[cfg(feature = "touch")]
    pub(crate) fn new(model: AppModel, touch: touch::Touch) -> Self {
        Self::with_navigation(model, NavigationInput::new(touch))
    }

    #[cfg(not(feature = "touch"))]
    pub(crate) fn new(model: AppModel) -> Self {
        Self::with_navigation(model, NavigationInput::new())
    }

    fn with_navigation(model: AppModel, navigation: NavigationInput) -> Self {
        let presented_view = model.active_view();
        Self {
            model,
            navigation,
            views: Views::new(),
            gui_surface: GuiSurface::new(),
            presented_view,
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
        let active_view = self.model.active_view();
        #[cfg(feature = "settings")]
        let mut settings_action = None;
        #[cfg(feature = "speaker-synth")]
        let mut speaker_action = None;

        let selected = {
            let navigation = &mut self.navigation;
            let views = &mut self.views;
            navigation.poll(|pointer| match active_view {
                #[cfg(feature = "settings")]
                ViewId::Settings => settings_action = views.settings.handle_pointer(pointer),
                #[cfg(feature = "speaker-synth")]
                ViewId::Speaker => speaker_action = views.speaker.handle_pointer(pointer),
                _ => {}
            })
        };

        #[cfg(feature = "settings")]
        if let Some(brightness) = settings_action {
            self.model.set_brightness(brightness);
        }
        #[cfg(feature = "speaker-synth")]
        if let Some(action) = speaker_action {
            self.model.apply_speaker_action(action);
        }
        if let Some(view) = selected {
            self.model.request_view(view);
        }
        self.model.update(now);

        let requested = self.model.active_view();
        (requested != self.presented_view).then_some(ViewTransition {
            from: self.presented_view,
            to: requested,
        })
    }

    pub(crate) fn apply_navigation(&mut self, transition: ViewTransition, display: &mut Display) {
        debug_assert_eq!(transition.from, self.presented_view);
        self.presented_view = transition.to;
        {
            let mut navigation_surface = display.surface(design::NAV_REGION);
            navigation::render(&mut navigation_surface, transition.to);
        }
        self.present_current_view(display);
    }

    pub(crate) fn render(&mut self, display: &mut Display) {
        match self.presented_view {
            #[cfg(feature = "network-demo")]
            ViewId::Network => {
                if let Some(snapshot) = self.model.take_network_display() {
                    let mut content = display.surface(design::CONTENT_REGION);
                    self.views.network.present(&mut self.gui_surface, &mut content, &snapshot);
                }
            }
            #[cfg(feature = "imu-worldview")]
            ViewId::Imu => {
                if let Some(imu) = self.model.take_imu_display() {
                    let mut content = display.surface(design::CONTENT_REGION);
                    self.views.imu.present(&mut self.gui_surface, &mut content, &imu);
                }
            }
            #[cfg(feature = "mic-waveform")]
            ViewId::Microphone => {
                if let Some(frame) = self.model.take_waveform_frame() {
                    let mut content = display.surface(design::CONTENT_REGION);
                    self.views.microphone.render_waveform(&mut content, &frame);
                }
            }
            #[cfg(feature = "speaker-synth")]
            ViewId::Speaker => {
                if let Some(state) = self.model.take_speaker_display() {
                    let mut content = display.surface(design::CONTENT_REGION);
                    self.views.speaker.present(&mut self.gui_surface, &mut content, state);
                }
            }
            #[cfg(feature = "camera-view")]
            ViewId::Camera => {}
            #[cfg(feature = "settings")]
            ViewId::Settings => {
                if let Some(settings) = self.model.take_settings_display() {
                    let mut content = display.surface(design::CONTENT_REGION);
                    self.views.settings.present(&mut self.gui_surface, &mut content, settings.brightness);
                }
            }
            #[cfg(feature = "log-view")]
            ViewId::Log => {
                let _ = self.present_log_if_dirty(display);
            }
        }
    }

    #[cfg(feature = "camera-view")]
    pub(crate) fn render_camera(&self, display: &mut Display, frame: &mut camera::Frame<'_>) {
        if self.presented_view == ViewId::Camera {
            let mut content = display.surface(design::CONTENT_REGION);
            self.views.camera.render(&mut content, frame);
        }
    }

    fn present_current_view(&mut self, display: &mut Display) {
        let mut content = display.surface(design::CONTENT_REGION);
        match self.presented_view {
            #[cfg(feature = "network-demo")]
            ViewId::Network => self.views.network.present_shell(&mut self.gui_surface, &mut content),
            #[cfg(feature = "imu-worldview")]
            ViewId::Imu => self.views.imu.present_shell(&mut self.gui_surface, &mut content),
            #[cfg(feature = "mic-waveform")]
            ViewId::Microphone => self.views.microphone.present_shell(&mut self.gui_surface, &mut content),
            #[cfg(feature = "speaker-synth")]
            ViewId::Speaker => {
                let state = self.model.take_speaker_display().expect("speaker state must be dirty when entering Speaker");
                self.views.speaker.present(&mut self.gui_surface, &mut content, state);
            }
            #[cfg(feature = "camera-view")]
            ViewId::Camera => self.views.camera.present_shell(&mut content),
            #[cfg(feature = "settings")]
            ViewId::Settings => {
                let state = self.model.take_settings_display().expect("settings state must be dirty when entering Settings");
                self.views.settings.present(&mut self.gui_surface, &mut content, state.brightness);
            }
            #[cfg(feature = "log-view")]
            ViewId::Log => {
                if !self.present_log_if_dirty_on_surface(&mut content) {
                    self.views.log.present_shell(&mut self.gui_surface, &mut content);
                }
            }
        }
    }

    #[cfg(feature = "log-view")]
    fn present_log_if_dirty(&mut self, display: &mut Display) -> bool {
        let mut content = display.surface(design::CONTENT_REGION);
        self.present_log_if_dirty_on_surface(&mut content)
    }

    #[cfg(feature = "log-view")]
    fn present_log_if_dirty_on_surface(
        &mut self,
        content: &mut crate::capabilities::display::Surface<'_>,
    ) -> bool {
        let model = &mut self.model;
        let log = &mut self.views.log;
        let gui = &mut self.gui_surface;
        model.with_log_text(|text| log.present(gui, content, text)).is_some()
    }
}
