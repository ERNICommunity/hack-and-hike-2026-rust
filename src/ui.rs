//! CPU0 presentation owner.
//!
//! `Ui` coordinates semantic application state, touch routing, semantic views,
//! and one fixed PSRAM embedded-gui surface. KDL owns content-view geometry;
//! `Display` remains the only LCD transport boundary.

mod design;
mod gui;
mod navigation;
mod views;

use embassy_time::Instant;

use crate::{
    display::Display,
    models::{AppModel, ViewId},
    service_inputs::TouchInput,
};

use gui::GuiSurface;
use navigation::NavigationInput;
use views::Views;

/// A semantic view transition prepared by input/model state and not yet fully
/// presented to the LCD.
#[derive(Clone, Copy, Debug)]
pub struct ViewTransition {
    pub from: ViewId,
    pub to: ViewId,
}

/// Exclusive CPU0 presentation state.
pub struct Ui {
    model: AppModel,
    navigation: NavigationInput,
    views: Views,
    gui_surface: GuiSurface,
    presented_view: ViewId,
}

impl Ui {
    pub fn new(model: AppModel, touch: TouchInput) -> Self {
        let presented_view = model.active_view();
        Self {
            model,
            navigation: NavigationInput::new(touch),
            views: Views::new(),
            gui_surface: GuiSurface::new(),
            presented_view,
        }
    }

    pub fn render_initial(&mut self, display: &mut Display) {
        navigation::render(display, self.presented_view);
        if self.presented_view == ViewId::Log && self.present_log_if_dirty(display) {
            return;
        }
        self.views.present_shell(
            self.presented_view,
            &mut self.gui_surface,
            display,
            None,
        );
    }

    /// Drain input, convert view interaction into semantic model actions, refresh
    /// the active model, and report a navigation transition still to present.
    pub fn prepare_frame(&mut self, now: Instant) -> Option<ViewTransition> {
        let active_view = self.model.active_view();
        let mut brightness_action = None;
        let selected = {
            let navigation = &mut self.navigation;
            let views = &mut self.views;
            navigation.poll(|pointer| {
                if let Some(brightness) = views.handle_pointer(active_view, pointer) {
                    brightness_action = Some(brightness);
                }
            })
        };

        if let Some(brightness) = brightness_action {
            self.model.set_brightness(brightness);
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

    /// Commit a prepared transition and present the destination immediately.
    pub fn apply_navigation(&mut self, transition: ViewTransition, display: &mut Display) {
        debug_assert_eq!(transition.from, self.presented_view);
        self.presented_view = transition.to;

        navigation::render(display, transition.to);

        // Log already owns a cached presentation snapshot and `request_view`
        // marks it dirty before this transition is applied. Present that content
        // directly so entering Log does not first transmit an empty shell and
        // then transmit the same full content rectangle again.
        if transition.to == ViewId::Log && self.present_log_if_dirty(display) {
            return;
        }

        let settings = if transition.to == ViewId::Settings {
            self.model.take_settings_display()
        } else {
            None
        };
        self.views.present_shell(
            transition.to,
            &mut self.gui_surface,
            display,
            settings,
        );
    }

    /// Render dirty dynamic data for the currently presented semantic view.
    pub fn render(&mut self, display: &mut Display) {
        match self.presented_view {
            ViewId::Network => {
                if let Some(snapshot) = self.model.take_network_display() {
                    self.views.present_network(
                        &mut self.gui_surface,
                        display,
                        &snapshot,
                    );
                }
            }
            ViewId::Imu => {
                if let Some(imu) = self.model.take_imu_display() {
                    self.views
                        .present_imu(&mut self.gui_surface, display, &imu);
                }
            }
            ViewId::Microphone => {
                if let Some(frame) = self.model.take_waveform_frame() {
                    self.views.render_microphone(display, &frame);
                }
            }
            ViewId::Speaker => {}
            ViewId::Settings => {
                if let Some(settings) = self.model.take_settings_display() {
                    self.views
                        .present_settings(&mut self.gui_surface, display, settings);
                }
            }
            ViewId::Log => {
                let _ = self.present_log_if_dirty(display);
            }
        }
    }

    fn present_log_if_dirty(&mut self, display: &mut Display) -> bool {
        let model = &mut self.model;
        let views = &mut self.views;
        let surface = &mut self.gui_surface;
        model
            .with_log_text(|text| views.present_log(surface, display, text))
            .is_some()
    }
}
