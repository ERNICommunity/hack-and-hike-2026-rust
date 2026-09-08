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

const CAMERA_IMAGE_WIDTH: usize = design::CONTENT_WIDTH;
const CAMERA_IMAGE_HEIGHT: usize = CAMERA_IMAGE_WIDTH * camera::HEIGHT / camera::WIDTH;
const CAMERA_IMAGE_Y: usize = (design::CONTENT_HEIGHT - CAMERA_IMAGE_HEIGHT) / 2;
const CAMERA_X_BYTE_OFFSETS: [u16; CAMERA_IMAGE_WIDTH] = build_camera_x_byte_offsets();
const CAMERA_Y_MAP: [u8; CAMERA_IMAGE_HEIGHT] = build_camera_y_map();

const _: () = assert!(CAMERA_IMAGE_WIDTH <= design::CONTENT_WIDTH);
const _: () = assert!(CAMERA_IMAGE_HEIGHT <= design::CONTENT_HEIGHT);
const _: () = assert!(CAMERA_IMAGE_WIDTH > 1 && CAMERA_IMAGE_HEIGHT > 1);
const _: () = assert!(camera::WIDTH * 2 - 2 <= u16::MAX as usize);
const _: () = assert!(camera::HEIGHT - 1 <= u8::MAX as usize);

const fn build_camera_x_byte_offsets() -> [u16; CAMERA_IMAGE_WIDTH] {
    let mut map = [0u16; CAMERA_IMAGE_WIDTH];
    let mut image_x = 0;
    while image_x < CAMERA_IMAGE_WIDTH {
        let source_x = image_x * (camera::WIDTH - 1) / (CAMERA_IMAGE_WIDTH - 1);
        map[image_x] = (source_x * 2) as u16;
        image_x += 1;
    }
    map
}

const fn build_camera_y_map() -> [u8; CAMERA_IMAGE_HEIGHT] {
    let mut map = [0u8; CAMERA_IMAGE_HEIGHT];
    let mut image_y = 0;
    while image_y < CAMERA_IMAGE_HEIGHT {
        map[image_y] =
            (image_y * (camera::HEIGHT - 1) / (CAMERA_IMAGE_HEIGHT - 1)) as u8;
        image_y += 1;
    }
    map
}

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

    pub(crate) fn render_camera(&self, display: &mut Display, frame: &camera::Frame<'_>) {
        // Keep the whole 4:3 QVGA image visible. The 44 px navigation rail leaves
        // 276x240 for content, so width is the limiting dimension: 320x240 scales
        // to 276x207. The remaining 33 vertical pixels are letterboxed.
        //
        // Both scaling axes are precomputed at compile time. The X table stores
        // RGB565 byte offsets directly, so the active-row hot loop performs no
        // coordinate division or multiply. Only the 33 letterbox rows are cleared.
        display.render_scanlines(design::CONTENT_REGION, |local_y, pixels| {
            if !(CAMERA_IMAGE_Y..CAMERA_IMAGE_Y + CAMERA_IMAGE_HEIGHT).contains(&local_y) {
                pixels.fill(theme::BLACK_RGB565);
                return;
            }

            let image_y = local_y - CAMERA_IMAGE_Y;
            let source_y = CAMERA_Y_MAP[image_y] as usize;
            let source = frame.scanline(source_y);

            for (pixel, &byte) in pixels[..CAMERA_IMAGE_WIDTH]
                .iter_mut()
                .zip(CAMERA_X_BYTE_OFFSETS.iter())
            {
                let byte = byte as usize;
                *pixel = u16::from_be_bytes([source[byte], source[byte + 1]]);
            }
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
    display.render_scanlines(design::CONTENT_REGION, |_local_y, pixels| {
        pixels.fill(theme::BLACK_RGB565);
    });
}
