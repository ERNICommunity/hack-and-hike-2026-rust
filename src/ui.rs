//! CPU0 presentation owner.
//!
//! `Ui` is the only type that combines application models, the sole touch-input
//! capability, navigation gesture state, and the fixed PSRAM content framebuffer.
//! It can request generic pixel submission from `Display`, but it cannot access
//! SPI/DMA/controller transport. The contained `AppModel` cannot access
//! presentation geometry or touch events.

mod design;
mod framebuffer;
mod navigation;
mod views;
mod waveform;

use embassy_time::Instant;

use crate::{
    display::Display,
    models::{AppModel, ViewId},
    service_inputs::TouchInput,
};

use framebuffer::ContentFramebuffer;
use navigation::NavigationInput;

/// A requested/presented view transition.
///
/// The value exists so bootstrap can instrument a transition without coupling
/// the memory diagnostics module to presentation types.
#[derive(Clone, Copy, Debug)]
pub struct NavigationChange {
    pub from: ViewId,
    pub to: ViewId,
}

/// Exclusive CPU0 presentation state and touch consumer.
pub struct Ui {
    model: AppModel,
    navigation: NavigationInput,
    content: ContentFramebuffer,
    presented_view: ViewId,
}

impl Ui {
    pub fn new(model: AppModel, touch: TouchInput) -> Self {
        let presented_view = model.active_view();
        Self {
            model,
            navigation: NavigationInput::new(touch),
            content: ContentFramebuffer::new(),
            presented_view,
        }
    }

    pub fn render_initial(&mut self, display: &mut Display) {
        navigation::render(display, self.presented_view);
        views::render_shell(&mut self.content, self.presented_view);
        self.blit_content(display);
    }

    /// Drain input, refresh the active model at its configured cadence, and
    /// report a view transition that still needs to be presented.
    pub fn prepare_frame(&mut self, now: Instant) -> Option<NavigationChange> {
        if let Some(view) = self.navigation.poll() {
            self.model.request_view(view);
        }
        self.model.update(now);

        let requested = self.model.active_view();
        (requested != self.presented_view).then_some(NavigationChange {
            from: self.presented_view,
            to: requested,
        })
    }

    /// Commit a prepared navigation transition and draw the destination shell.
    pub fn apply_navigation(&mut self, change: NavigationChange, display: &mut Display) {
        debug_assert_eq!(change.from, self.presented_view);
        self.presented_view = change.to;

        navigation::render(display, change.to);
        views::render_shell(&mut self.content, change.to);
        self.blit_content(display);
    }

    /// Render only dirty data for the currently presented view.
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
                    waveform::render(display, &frame);
                }
            }
            ViewId::Sound => {}
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
