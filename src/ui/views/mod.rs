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

const CAMERA_CROP_PIXELS: usize = camera::WIDTH - design::CONTENT_WIDTH;
const CAMERA_CROP_LEFT: usize = CAMERA_CROP_PIXELS / 2;
const CAMERA_CROP_RIGHT: usize = CAMERA_CROP_PIXELS - CAMERA_CROP_LEFT;
const CAMERA_SOURCE_START_BYTE: usize = CAMERA_CROP_LEFT * 2;
const CAMERA_SOURCE_END_BYTE: usize = (camera::WIDTH - CAMERA_CROP_RIGHT) * 2;

const _: () = assert!(camera::HEIGHT == design::CONTENT_HEIGHT);
const _: () = assert!(camera::WIDTH >= design::CONTENT_WIDTH);
const _: () = assert!(CAMERA_CROP_PIXELS % 2 == 0);
const _: () = assert!(CAMERA_SOURCE_END_BYTE - CAMERA_SOURCE_START_BYTE == design::CONTENT_WIDTH * 2);

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
        // QVGA is already 240 px tall, so keep native vertical resolution and
        // crop 22 px from each horizontal edge to fill the 276x240 content area.
        // GC0308 hardware rotation already puts scanlines in display order.
        //
        // Sensor RGB565 bytes are big-endian, exactly matching the ILI9342C SPI
        // memory-write order. Copy them directly into the display's four-row DMA
        // batches instead of decoding to u16 and immediately encoding back to the
        // same two bytes for every pixel.
        let _ = display.render_rgb565_be_scanlines(design::CONTENT_REGION, |_local_y, bytes| {
            let Some(source) = frame.next_scanline() else {
                return false;
            };
            let cropped = &source[CAMERA_SOURCE_START_BYTE..CAMERA_SOURCE_END_BYTE];
            bytes.copy_from_slice(cropped);
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
        self.speaker.present(surface, display, state);
    }

    pub(crate) fn present_settings(
        &mut self,
        surface: &mut GuiSurface,
        display: &mut Display,
        state: SettingsDisplay,
    ) {
        self.settings.sync_brightness(state.brightness);
        self.settings.present(surface, display, state);
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
    display.render_scanlines(design::CONTENT_REGION, |_local_y, pixels| {
        pixels.fill(theme::BLACK_RGB565);
    });
}
