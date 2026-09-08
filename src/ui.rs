//! CPU0 presentation owner.
//!
//! `Ui` coordinates semantic application state, touch routing, view ownership,
//! the legacy PSRAM content framebuffer, and the fixed embedded-gui surface.
//! Views cannot access service hardware; `Display` remains the only LCD boundary.

mod design;
mod framebuffer;
mod gui;
mod navigation;
mod views;

use embassy_time::Instant;

use crate::{
    display::Display,
    models::{AppModel, ViewId},
    service_inputs::TouchInput,
};

use framebuffer::ContentFramebuffer;
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
    content: ContentFramebuffer,
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
            content: ContentFramebuffer::new(),
            gui_surface: GuiSurface::new(),
            presented_view,
        }
    }

    pub fn render_initial(&mut self, display: &mut Display) {
        navigation::render(display, self.presented_view);
        self.views.present_shell(
            self.presented_view,
            &mut self.content,
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

    /// Commit a prepared transition and present the destination's static shell.
    pub fn apply_navigation(&mut self, transition: ViewTransition, display: &mut Display) {
        debug_assert_eq!(transition.from, self.presented_view);
        self.presented_view = transition.to;

        navigation::render(display, transition.to);
        let settings = if transition.to == ViewId::Settings {
            self.model.take_settings_display()
        } else {
            None
        };
        self.views.present_shell(
            transition.to,
            &mut self.content,
            &mut self.gui_surface,
            display,
            settings,
        );
    }

    /// Render dirty dynamic data for the currently presented view.
    pub fn render(&mut self, display: &mut Display) {
        match self.presented_view {
            ViewId::Network => {
                if let Some(snapshot) = self.model.take_network_display() {
                    views::render_network(&mut self.content, &snapshot);
                    self.blit_content(display);
                }
            }
            ViewId::Imu => {
                if let Some(imu) = self.model.take_imu_display() {
                    views::render_imu(&mut self.content, &imu);
                    self.blit_content(display);
                }
            }
            ViewId::Microphone => {
                if let Some(frame) = self.model.take_waveform_frame() {
                    self.views.render_microphone(display, &frame);
                }
            }
            ViewId::Settings => {
                if let Some(settings) = self.model.take_settings_display() {
                    self.views
                        .present_settings(&mut self.gui_surface, display, settings);
                }
            }
            ViewId::Speaker => {}
            ViewId::Log => {
                let rendered = {
                    let model = &mut self.model;
                    let content = &mut self.content;
                    model
                        .with_log_text(|text| views::render_log(content, text))
                        .is_some()
                };
                if rendered {
                    self.blit_content(display);
                }
            }
        }
    }

    fn blit_content(&self, display: &mut Display) {
        display.blit(design::CONTENT_REGION, self.content.pixels());
    }
}
