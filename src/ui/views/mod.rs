//! Semantic view ownership and dispatch.
//!
//! Every screen owns a KDL-backed fixed-capacity context in its matching module.
//! KDL is the single source of normal page geometry. View-specific overlays stay
//! private to their semantic view; only genuinely reusable drawing primitives
//! live in `common`.

mod common;
mod imu;
mod log;
mod microphone;
mod network;
mod settings;
mod speaker;

use crate::{
    audio::{PitchSemitones, TempoBpm},
    camera,
    display::Display,
    display_control::BrightnessPercent,
    models::{ImuDisplay, SettingsDisplay, SpeakerDisplay, ViewId},
    network as network_service, theme,
    waveform::WaveformFrame,
};

use super::{design, gui::GuiSurface, navigation::ContentPointer};

// The camera path deliberately writes a smaller 4:3 viewport than the full
// 276x240 content area. A full content update takes about 26.5 ms at the proven
// 40 MHz LCD SPI rate, longer than one normal panel refresh. Scaling QVGA to 70%
// in both axes reduces the live write to 224x168 = 75,264 bytes, about 15.1 ms,
// while preserving the complete camera field of view.
const CAMERA_SAMPLE_SOURCE_SPAN: usize = 10;
const CAMERA_SAMPLE_OUTPUT_SPAN: usize = 7;
const CAMERA_SAMPLE_OFFSETS: [usize; CAMERA_SAMPLE_OUTPUT_SPAN] = [0, 1, 3, 4, 6, 7, 9];
const CAMERA_VIEW_WIDTH: usize =
    camera::WIDTH / CAMERA_SAMPLE_SOURCE_SPAN * CAMERA_SAMPLE_OUTPUT_SPAN;
const CAMERA_VIEW_HEIGHT: usize =
    camera::HEIGHT / CAMERA_SAMPLE_SOURCE_SPAN * CAMERA_SAMPLE_OUTPUT_SPAN;
const CAMERA_VIEW_X: usize = (design::CONTENT_WIDTH - CAMERA_VIEW_WIDTH) / 2;
const CAMERA_VIEW_Y: usize = (design::CONTENT_HEIGHT - CAMERA_VIEW_HEIGHT) / 2;
const CAMERA_VIEW_REGION: crate::display::Region = design::ContentRect::new(
    CAMERA_VIEW_X,
    CAMERA_VIEW_Y,
    CAMERA_VIEW_WIDTH,
    CAMERA_VIEW_HEIGHT,
)
.screen_region();

const _: () = assert!(camera::WIDTH % CAMERA_SAMPLE_SOURCE_SPAN == 0);
const _: () = assert!(camera::HEIGHT % CAMERA_SAMPLE_SOURCE_SPAN == 0);
const _: () = assert!(CAMERA_VIEW_WIDTH <= design::CONTENT_WIDTH);
const _: () = assert!(CAMERA_VIEW_HEIGHT <= design::CONTENT_HEIGHT);
const _: () = assert!(CAMERA_VIEW_WIDTH * CAMERA_SAMPLE_SOURCE_SPAN == camera::WIDTH * CAMERA_SAMPLE_OUTPUT_SPAN);
const _: () = assert!(CAMERA_VIEW_HEIGHT * CAMERA_SAMPLE_SOURCE_SPAN == camera::HEIGHT * CAMERA_SAMPLE_OUTPUT_SPAN);

pub(crate) enum Interaction {
    SetBrightness(BrightnessPercent),
    ToggleSpeakerPlayback,
    PlaySpeakerOneShot,
    SetSpeakerTempo(TempoBpm),
    SetSpeakerPitch(PitchSemitones),
}

pub(crate) struct Views {
    network: network::View,
    imu: imu::View,
    microphone: microphone::View,
    speaker: speaker::View,
    settings: settings::View,
    log: log::View,
}

impl Views {
    pub(crate) fn new() -> Self {
        Self {
            network: network::View::new(),
            imu: imu::View::new(),
            microphone: microphone::View::new(),
            speaker: speaker::View::new(),
            settings: settings::View::new(BrightnessPercent::FULL),
            log: log::View::new(),
        }
    }

    pub(crate) fn present_shell(
        &mut self,
        view: ViewId,
        surface: &mut GuiSurface,
        display: &mut Display,
        settings_display: Option<SettingsDisplay>,
    ) {
        match view {
            ViewId::Network => self.network.present_shell(surface, display),
            ViewId::Imu => self.imu.present_shell(surface, display),
            ViewId::Microphone => self.microphone.present_shell(surface, display),
            ViewId::Speaker => self.speaker.present(surface, display),
            ViewId::Camera => present_camera_shell(display),
            ViewId::Settings => {
                if let Some(state) = settings_display {
                    self.settings.sync_brightness(state.brightness);
                }
                self.settings.present(surface, display);
            }
            ViewId::Log => self.log.present_shell(surface, display),
        }
    }

    pub(crate) fn handle_pointer(
        &mut self,
        view: ViewId,
        pointer: ContentPointer,
    ) -> Option<Interaction> {
        match view {
            ViewId::Settings => self
                .settings
                .handle_pointer(pointer)
                .map(Interaction::SetBrightness),
            ViewId::Speaker => self.speaker.handle_pointer(pointer).map(|action| match action {
                speaker::Action::TogglePlayback => Interaction::ToggleSpeakerPlayback,
                speaker::Action::PlayOneShot => Interaction::PlaySpeakerOneShot,
                speaker::Action::SetTempo(tempo) => Interaction::SetSpeakerTempo(tempo),
                speaker::Action::SetPitch(pitch) => Interaction::SetSpeakerPitch(pitch),
            }),
            _ => None,
        }
    }

    pub(crate) fn present_network(
        &mut self,
        surface: &mut GuiSurface,
        display: &mut Display,
        snapshot: &network_service::Snapshot,
    ) {
        self.network.present(surface, display, snapshot);
    }

    pub(crate) fn present_imu(
        &mut self,
        surface: &mut GuiSurface,
        display: &mut Display,
        state: &ImuDisplay,
    ) {
        self.imu.present(surface, display, state);
    }

    pub(crate) fn render_microphone(&self, display: &mut Display, frame: &WaveformFrame) {
        self.microphone.render_waveform(display, frame);
    }

    pub(crate) fn render_camera(&self, display: &mut Display, frame: &mut camera::Frame<'_>) {
        // GC0308 hardware rotation already puts QVGA scanlines in display order.
        // Keep the whole sensor field of view, but decimate each 10-pixel/line
        // group to seven samples. The fixed pattern avoids scaling state or a
        // framebuffer and consumes the final source line, so the existing VSYNC
        // frame validation remains unchanged.
        let mut next_source_y = 0usize;
        let _ = display.render_rgb565_be_scanlines(CAMERA_VIEW_REGION, |local_y, bytes| {
            let source_group = local_y / CAMERA_SAMPLE_OUTPUT_SPAN;
            let source_offset = CAMERA_SAMPLE_OFFSETS[local_y % CAMERA_SAMPLE_OUTPUT_SPAN];
            let target_source_y = source_group * CAMERA_SAMPLE_SOURCE_SPAN + source_offset;

            while next_source_y < target_source_y {
                if frame.next_scanline().is_none() {
                    return false;
                }
                next_source_y += 1;
            }

            let Some(source) = frame.next_scanline() else {
                return false;
            };
            next_source_y += 1;

            for group in 0..(camera::WIDTH / CAMERA_SAMPLE_SOURCE_SPAN) {
                let source_base = group * CAMERA_SAMPLE_SOURCE_SPAN * 2;
                let output_base = group * CAMERA_SAMPLE_OUTPUT_SPAN * 2;

                for (output_pixel, source_pixel) in
                    CAMERA_SAMPLE_OFFSETS.iter().copied().enumerate()
                {
                    let src = source_base + source_pixel * 2;
                    let dst = output_base + output_pixel * 2;
                    bytes[dst] = source[src];
                    bytes[dst + 1] = source[src + 1];
                }
            }

            true
        });
    }

    pub(crate) fn present_speaker(
        &mut self,
        surface: &mut GuiSurface,
        display: &mut Display,
        state: SpeakerDisplay,
    ) {
        self.speaker.sync(state);
        self.speaker.present(surface, display);
    }

    pub(crate) fn present_settings(
        &mut self,
        surface: &mut GuiSurface,
        display: &mut Display,
        state: SettingsDisplay,
    ) {
        self.settings.sync_brightness(state.brightness);
        self.settings.present(surface, display);
    }

    pub(crate) fn present_log(
        &mut self,
        surface: &mut GuiSurface,
        display: &mut Display,
        contents: &str,
    ) {
        self.log.present(surface, display, contents);
    }
}

fn present_camera_shell(display: &mut Display) {
    // Clear the entire content area once when entering Camera. Steady-state
    // frames only rewrite the centered 224x168 live viewport, leaving stable
    // black letterboxing around it and minimizing bytes sent while moving.
    display.render_scanlines(design::CONTENT_REGION, |_local_y, pixels| {
        pixels.fill(theme::BLACK_RGB565);
    });
}
