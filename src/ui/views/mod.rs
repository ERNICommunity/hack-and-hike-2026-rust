//! Semantic view ownership and dispatch.
//!
//! Each screen owns its presentation code in a matching module. KDL-backed
//! screens also own their fixed-capacity `embedded-gui` context here; reusable
//! framebuffer/text primitives remain in `common`.

mod common;
mod imu;
mod log;
mod microphone;
mod network;
mod settings;
mod speaker;

use crate::{
    display::Display,
    display_control::{BrightnessPercent},
    models::{ImuDisplay, SettingsDisplay, ViewId},
    network as network_service,
    waveform::WaveformFrame,
};

use super::{
    design,
    framebuffer::ContentFramebuffer,
    gui::GuiSurface,
    navigation::ContentPointer,
};

pub(crate) struct Views {
    microphone: microphone::View,
    settings: settings::View,
}

impl Views {
    pub(crate) fn new() -> Self {
        Self {
            microphone: microphone::View::new(),
            settings: settings::View::new(BrightnessPercent::FULL),
        }
    }

    /// Present the complete static shell for a destination view.
    pub(crate) fn present_shell(
        &mut self,
        view: ViewId,
        content: &mut ContentFramebuffer,
        gui_surface: &mut GuiSurface,
        display: &mut Display,
        settings_display: Option<SettingsDisplay>,
    ) {
        match view {
            ViewId::Network => {
                network::render_shell(content);
                blit_content(content, display);
            }
            ViewId::Imu => {
                imu::render_shell(content);
                blit_content(content, display);
            }
            ViewId::Microphone => self.microphone.present_shell(gui_surface, display),
            ViewId::Speaker => {
                speaker::render_shell(content);
                blit_content(content, display);
            }
            ViewId::Settings => {
                if let Some(state) = settings_display {
                    self.settings.sync_brightness(state.brightness);
                }
                self.settings.present(gui_surface, display);
            }
            ViewId::Log => {
                log::render_shell(content);
                blit_content(content, display);
            }
        }
    }

    pub(crate) fn handle_pointer(
        &mut self,
        view: ViewId,
        pointer: ContentPointer,
    ) -> Option<BrightnessPercent> {
        match view {
            ViewId::Settings => self.settings.handle_pointer(pointer),
            _ => None,
        }
    }

    pub(crate) fn present_settings(
        &mut self,
        gui_surface: &mut GuiSurface,
        display: &mut Display,
        state: SettingsDisplay,
    ) {
        self.settings.sync_brightness(state.brightness);
        self.settings.present(gui_surface, display);
    }

    pub(crate) fn render_microphone(&self, display: &mut Display, frame: &WaveformFrame) {
        self.microphone.render_waveform(display, frame);
    }
}

pub(crate) fn render_network(
    frame: &mut ContentFramebuffer,
    snapshot: &network_service::Snapshot,
) {
    network::render(frame, snapshot);
}

pub(crate) fn render_imu(frame: &mut ContentFramebuffer, display: &ImuDisplay) {
    imu::render(frame, display);
}

pub(crate) fn render_log(frame: &mut ContentFramebuffer, contents: &str) {
    log::render(frame, contents);
}

fn blit_content(content: &ContentFramebuffer, display: &mut Display) {
    display.blit(design::CONTENT_REGION, content.pixels());
}
