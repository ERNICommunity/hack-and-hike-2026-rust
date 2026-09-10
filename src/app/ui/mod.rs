//! CPU0 presentation owner.
//!
//! `Ui` coordinates semantic application state, touch routing, semantic views,
//! and one fixed PSRAM embedded-gui surface. KDL owns content-view geometry;
//! `Display` remains the only LCD transport boundary.

mod design;
mod gui;
mod navigation;
mod theme;
mod views;

use embassy_time::Instant;

use crate::{
    app::model::{AppModel, ViewId},
    services::{camera, display::Display, touch},
};

use gui::GuiSurface;
use navigation::NavigationInput;
use views::{SpeakerAction, Views};

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
    pub(crate) fn new(model: AppModel, touch: touch::Input) -> Self {
        let presented_view = model.active_view();
        Self {
            model,
            navigation: NavigationInput::new(touch),
            views: Views::new(),
            gui_surface: GuiSurface::new(),
            presented_view,
        }
    }

    pub(crate) fn presented_view(&self) -> ViewId {
        self.presented_view
    }

    pub(crate) fn render_initial(&mut self, display: &mut Display) {
        navigation::render(display, self.presented_view);
        self.present_current_view(display);
    }

    pub(crate) fn prepare_frame(&mut self, now: Instant) -> Option<ViewTransition> {
        let active_view = self.model.active_view();
        let mut settings_action = None;
        let mut speaker_action = None;
        let selected = {
            let navigation = &mut self.navigation;
            let views = &mut self.views;
            navigation.poll(|pointer| match active_view {
                ViewId::Settings => settings_action = views.settings.handle_pointer(pointer),
                ViewId::Speaker => speaker_action = views.speaker.handle_pointer(pointer),
                _ => {}
            })
        };

        if let Some(brightness) = settings_action {
            self.model.set_brightness(brightness);
        }
        if let Some(action) = speaker_action {
            match action {
                SpeakerAction::TogglePlayback => self.model.toggle_speaker_playback(),
                SpeakerAction::PlayOneShot => self.model.play_speaker_one_shot(),
                SpeakerAction::SetTempo(tempo) => self.model.set_speaker_tempo(tempo),
                SpeakerAction::SetPitch(pitch) => self.model.set_speaker_pitch(pitch),
            }
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
        navigation::render(display, transition.to);
        self.present_current_view(display);
    }

    pub(crate) fn render(&mut self, display: &mut Display) {
        match self.presented_view {
            ViewId::Network => {
                if let Some(snapshot) = self.model.take_network_display() {
                    self.views
                        .network
                        .present(&mut self.gui_surface, display, &snapshot);
                }
            }
            ViewId::Imu => {
                if let Some(imu) = self.model.take_imu_display() {
                    self.views.imu.present(&mut self.gui_surface, display, &imu);
                }
            }
            ViewId::Microphone => {
                if let Some(frame) = self.model.take_waveform_frame() {
                    self.views.microphone.render_waveform(display, &frame);
                }
            }
            ViewId::Speaker => {
                if let Some(state) = self.model.take_speaker_display() {
                    self.views
                        .speaker
                        .present(&mut self.gui_surface, display, state);
                }
            }
            ViewId::Camera => {}
            ViewId::Settings => {
                if let Some(settings) = self.model.take_settings_display() {
                    self.views.settings.present(
                        &mut self.gui_surface,
                        display,
                        settings.brightness,
                    );
                }
            }
            ViewId::Log => {
                let _ = self.present_log_if_dirty(display);
            }
        }
    }

    pub(crate) fn render_camera(&self, display: &mut Display, frame: &mut camera::Frame<'_>) {
        if self.presented_view == ViewId::Camera {
            self.views.camera.render(display, frame);
        }
    }

    fn present_current_view(&mut self, display: &mut Display) {
        match self.presented_view {
            ViewId::Network => self
                .views
                .network
                .present_shell(&mut self.gui_surface, display),
            ViewId::Imu => self.views.imu.present_shell(&mut self.gui_surface, display),
            ViewId::Microphone => self
                .views
                .microphone
                .present_shell(&mut self.gui_surface, display),
            ViewId::Speaker => {
                let state = self
                    .model
                    .take_speaker_display()
                    .expect("speaker state must be dirty when entering Speaker");
                self.views
                    .speaker
                    .present(&mut self.gui_surface, display, state);
            }
            ViewId::Camera => self.views.camera.present_shell(display),
            ViewId::Settings => {
                let state = self
                    .model
                    .take_settings_display()
                    .expect("settings state must be dirty when entering Settings");
                self.views
                    .settings
                    .present(&mut self.gui_surface, display, state.brightness);
            }
            ViewId::Log => {
                if !self.present_log_if_dirty(display) {
                    self.views.log.present_shell(&mut self.gui_surface, display);
                }
            }
        }
    }

    fn present_log_if_dirty(&mut self, display: &mut Display) -> bool {
        let model = &mut self.model;
        let log = &mut self.views.log;
        let surface = &mut self.gui_surface;
        model
            .with_log_text(|text| log.present(surface, display, text))
            .is_some()
    }
}
