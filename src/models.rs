//! CPU0 application models.
//!
//! `AppModel` owns bounded presentation-sized state and the CPU0 reader/command
//! handles required to refresh or mutate it. It does not know display geometry,
//! fonts, touch gestures, SPI/DMA, LCD controller details, or PMIC registers.

use embassy_time::{Duration, Instant};

use crate::{
    audio, data_plane,
    display_control::{BrightnessControl, BrightnessPercent},
    imu, logger, network,
    service_inputs::{AudioInput, ImuInput, NetworkInput},
    waveform::{MAX_AMPLITUDE_PIXELS, POINTS, WaveformFrame},
};

const WAVEFORM_UPDATE: Duration = Duration::from_millis(32);
const WAVEFORM_PEAK_FLOOR: u16 = 1024;
const IMU_UPDATE: Duration = Duration::from_millis(40);
const NETWORK_UPDATE: Duration = Duration::from_millis(200);
const LOG_REFRESH: Duration = Duration::from_millis(100);

/// Semantic page identity shared by application refresh policy and presentation.
///
/// There is deliberately no numeric representation or presentation ordering on
/// this type. Navigation order belongs to the UI design.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewId {
    Network,
    Imu,
    Microphone,
    Speaker,
    Settings,
    Log,
}

impl ViewId {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Network => "Network",
            Self::Imu => "Imu",
            Self::Microphone => "Microphone",
            Self::Speaker => "Speaker",
            Self::Settings => "Settings",
            Self::Log => "Log",
        }
    }
}

/// CPU1 data readers consumed directly by `AppModel`.
pub struct AppModelInputs {
    pub network: NetworkInput,
    pub imu: ImuInput,
    pub audio: AudioInput,
}

struct NetworkModel {
    input: NetworkInput,
    display: Option<network::Snapshot>,
    last_revision: u32,
    last_update: Instant,
    dirty: bool,
}

impl NetworkModel {
    fn new(input: NetworkInput) -> Self {
        Self {
            input,
            display: None,
            last_revision: u32::MAX,
            last_update: Instant::now(),
            dirty: false,
        }
    }

    fn mark_dirty(&mut self) {
        if self.display.is_some() {
            self.dirty = true;
        }
    }

    fn update_if_due(&mut self, now: Instant) {
        if now - self.last_update < NETWORK_UPDATE {
            return;
        }
        self.last_update = now;

        let Some(snapshot) = self.input.take_latest() else {
            return;
        };
        if snapshot.revision == self.last_revision {
            return;
        }
        self.last_revision = snapshot.revision;
        self.display = Some(snapshot);
        self.dirty = true;
    }

    fn take_display(&mut self) -> Option<network::Snapshot> {
        if !self.dirty {
            return None;
        }
        self.dirty = false;
        self.display
    }
}

/// Rounded, presentation-sized IMU value owned by the CPU0 model.
#[derive(Clone, Copy, Debug)]
pub struct ImuDisplay {
    pub roll_deg: i32,
    pub pitch_deg: i32,
    pub yaw_deg: i32,
    pub status: imu::Status,
    pub read_errors: u32,
    pub mag_errors: u32,
    pub mag_status: imu::MagStatus,
    pub mag_field_ut: i32,
    pub mag_calibration: u8,
    pub gyro_bias_ready: bool,
}

struct ImuModel {
    input: ImuInput,
    display: ImuDisplay,
    last_revision: u32,
    last_update: Instant,
    dirty: bool,
}

impl ImuModel {
    fn new(input: ImuInput) -> Self {
        Self {
            input,
            display: ImuDisplay {
                roll_deg: 0,
                pitch_deg: 0,
                yaw_deg: 0,
                status: imu::Status::Starting,
                read_errors: 0,
                mag_errors: 0,
                mag_status: imu::MagStatus::Missing,
                mag_field_ut: 0,
                mag_calibration: 0,
                gyro_bias_ready: false,
            },
            last_revision: 0,
            last_update: Instant::now(),
            dirty: true,
        }
    }

    fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    fn update_if_due(&mut self, now: Instant) {
        if now - self.last_update < IMU_UPDATE {
            return;
        }
        self.last_update = now;

        let Some(snapshot) = self.input.take_latest() else {
            return;
        };
        if snapshot.revision == self.last_revision {
            return;
        }
        self.last_revision = snapshot.revision;

        self.display = ImuDisplay {
            roll_deg: round_units(snapshot.orientation.roll_deg),
            pitch_deg: round_units(snapshot.orientation.pitch_deg),
            yaw_deg: round_units(snapshot.orientation.yaw_deg),
            status: snapshot.status,
            read_errors: snapshot.read_errors,
            mag_errors: snapshot.mag_errors,
            mag_status: snapshot.mag_status,
            mag_field_ut: round_units(snapshot.mag_field_ut),
            mag_calibration: snapshot.mag_calibration_percent,
            gyro_bias_ready: snapshot.gyro_bias_ready,
        };
        self.dirty = true;
    }

    fn take_display(&mut self) -> Option<ImuDisplay> {
        if !self.dirty {
            return None;
        }
        self.dirty = false;
        Some(self.display)
    }
}

fn round_units(value: f32) -> i32 {
    if value >= 0.0 {
        (value + 0.5) as i32
    } else {
        (value - 0.5) as i32
    }
}

struct WaveformModel {
    input: AudioInput,
    frame: WaveformFrame,
    samples: [i16; audio::BLOCK_SAMPLES],
    last_sequence: u32,
    last_update: Instant,
    dirty: bool,
}

impl WaveformModel {
    fn new(input: AudioInput) -> Self {
        Self {
            input,
            frame: WaveformFrame::silent(),
            samples: [0; audio::BLOCK_SAMPLES],
            last_sequence: 0,
            last_update: Instant::now(),
            dirty: true,
        }
    }

    fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    fn update_if_due(&mut self, now: Instant) {
        if now - self.last_update < WAVEFORM_UPDATE {
            return;
        }
        self.last_update = now;

        let Some(info) = self.input.copy_latest_interleaved(&mut self.samples) else {
            return;
        };
        if info.sequence == self.last_sequence {
            return;
        }
        self.last_sequence = info.sequence;

        if self.update_frame(info) {
            self.dirty = true;
        }
    }

    fn update_frame(&mut self, info: audio::AudioBlockInfo) -> bool {
        const FRAMES_PER_POINT: usize = audio::BLOCK_FRAMES / POINTS;

        let left_scale = i32::from(info.peak_left.max(WAVEFORM_PEAK_FLOOR));
        let right_scale = i32::from(info.peak_right.max(WAVEFORM_PEAK_FLOOR));
        let mut changed = false;

        for point in 0..POINTS {
            let first_frame = point * FRAMES_PER_POINT;
            let last_frame = first_frame + FRAMES_PER_POINT;
            let mut left_sample = 0i16;
            let mut right_sample = 0i16;
            let mut left_magnitude = 0u16;
            let mut right_magnitude = 0u16;

            for frame in first_frame..last_frame {
                let sample_index = frame * audio::CHANNELS;
                let left = self.samples[sample_index];
                let right = self.samples[sample_index + 1];

                let left_abs = left.unsigned_abs();
                if left_abs > left_magnitude {
                    left_magnitude = left_abs;
                    left_sample = left;
                }

                let right_abs = right.unsigned_abs();
                if right_abs > right_magnitude {
                    right_magnitude = right_abs;
                    right_sample = right;
                }
            }

            let left_pixel = quantize_waveform(left_sample, left_scale);
            let right_pixel = quantize_waveform(right_sample, right_scale);

            if left_pixel != self.frame.left[point] {
                self.frame.left[point] = left_pixel;
                changed = true;
            }
            if right_pixel != self.frame.right[point] {
                self.frame.right[point] = right_pixel;
                changed = true;
            }
        }

        changed
    }

    fn take_frame(&mut self) -> Option<WaveformFrame> {
        if !self.dirty {
            return None;
        }
        self.dirty = false;
        Some(self.frame)
    }
}

fn quantize_waveform(sample: i16, scale: i32) -> i8 {
    ((i32::from(sample) * MAX_AMPLITUDE_PIXELS) / scale)
        .clamp(-MAX_AMPLITUDE_PIXELS, MAX_AMPLITUDE_PIXELS) as i8
}

/// Semantic state displayed by the Settings view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SettingsDisplay {
    pub brightness: BrightnessPercent,
}

struct SettingsModel {
    control: BrightnessControl,
    display: SettingsDisplay,
    dirty: bool,
}

impl SettingsModel {
    fn new(control: BrightnessControl) -> Self {
        Self {
            control,
            display: SettingsDisplay {
                brightness: BrightnessPercent::FULL,
            },
            dirty: true,
        }
    }

    fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    fn set_brightness(&mut self, brightness: BrightnessPercent) {
        if brightness == self.display.brightness {
            return;
        }
        self.display.brightness = brightness;
        self.control.set(brightness);
        self.dirty = true;
    }

    fn take_display(&mut self) -> Option<SettingsDisplay> {
        if !self.dirty {
            return None;
        }
        self.dirty = false;
        Some(self.display)
    }
}

/// Fixed-size MCU log snapshot.
struct LogModel {
    bytes: data_plane::FixedPsramBuffer<u8>,
    len: usize,
    revision: u32,
    last_check: Instant,
    dirty: bool,
}

impl LogModel {
    fn new() -> Self {
        Self {
            bytes: data_plane::FixedPsramBuffer::filled(logger::HISTORY_BYTES, 0),
            len: 0,
            revision: u32::MAX,
            last_check: Instant::now(),
            dirty: true,
        }
    }

    fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    fn update_if_due(&mut self, now: Instant) {
        if now - self.last_check < LOG_REFRESH {
            return;
        }
        self.last_check = now;
        self.refresh();
    }

    fn refresh(&mut self) {
        if logger::revision() == self.revision {
            return;
        }
        let Some((logs, revision)) = logger::snapshot(self.bytes.as_mut_slice()) else {
            return;
        };
        self.len = logs.len();
        self.revision = revision;
        self.dirty = true;
    }

    fn with_text<R>(&mut self, render: impl FnOnce(&str) -> R) -> Option<R> {
        if !self.dirty {
            return None;
        }
        self.dirty = false;
        let bytes = self.bytes.as_slice().get(..self.len)?;
        let text = core::str::from_utf8(bytes).ok()?;
        Some(render(text))
    }
}

/// Complete CPU0 application model.
pub struct AppModel {
    active_view: ViewId,
    network: NetworkModel,
    imu: ImuModel,
    waveform: WaveformModel,
    settings: SettingsModel,
    log: LogModel,
}

impl AppModel {
    pub fn new(inputs: AppModelInputs, brightness: BrightnessControl) -> Self {
        let AppModelInputs {
            network,
            imu,
            audio,
        } = inputs;
        let mut log = LogModel::new();
        log.refresh();

        Self {
            active_view: ViewId::Log,
            network: NetworkModel::new(network),
            imu: ImuModel::new(imu),
            waveform: WaveformModel::new(audio),
            settings: SettingsModel::new(brightness),
            log,
        }
    }

    pub fn active_view(&self) -> ViewId {
        self.active_view
    }

    pub fn request_view(&mut self, view: ViewId) {
        if view == self.active_view {
            return;
        }
        self.active_view = view;
        match view {
            ViewId::Network => self.network.mark_dirty(),
            ViewId::Imu => self.imu.mark_dirty(),
            ViewId::Microphone => self.waveform.mark_dirty(),
            ViewId::Settings => self.settings.mark_dirty(),
            ViewId::Log => self.log.mark_dirty(),
            ViewId::Speaker => {}
        }
    }

    pub fn update(&mut self, now: Instant) {
        match self.active_view {
            ViewId::Network => self.network.update_if_due(now),
            ViewId::Imu => self.imu.update_if_due(now),
            ViewId::Microphone => self.waveform.update_if_due(now),
            ViewId::Log => self.log.update_if_due(now),
            ViewId::Settings | ViewId::Speaker => {}
        }
    }

    pub fn set_brightness(&mut self, brightness: BrightnessPercent) {
        if self.active_view == ViewId::Settings {
            self.settings.set_brightness(brightness);
        }
    }

    pub fn take_settings_display(&mut self) -> Option<SettingsDisplay> {
        if self.active_view != ViewId::Settings {
            return None;
        }
        self.settings.take_display()
    }

    pub fn take_network_display(&mut self) -> Option<network::Snapshot> {
        if self.active_view != ViewId::Network {
            return None;
        }
        self.network.take_display()
    }

    pub fn with_log_text<R>(&mut self, render: impl FnOnce(&str) -> R) -> Option<R> {
        if self.active_view != ViewId::Log {
            return None;
        }
        self.log.with_text(render)
    }

    pub fn take_imu_display(&mut self) -> Option<ImuDisplay> {
        if self.active_view != ViewId::Imu {
            return None;
        }
        self.imu.take_display()
    }

    pub fn take_waveform_frame(&mut self) -> Option<WaveformFrame> {
        if self.active_view != ViewId::Microphone {
            return None;
        }
        self.waveform.take_frame()
    }
}
