//! CPU0 application models.
//!
//! This module owns state and data refresh policy. It does not own a Slint
//! window/component tree. `ui.rs` is only a presentation adapter around these
//! models.

use alloc::rc::Rc;
use core::cell::{Cell, RefCell};

use embassy_time::{Duration, Instant};
use slint::{Model, ModelNotify, ModelTracker};

use crate::{
    audio, data_plane, imu, logger,
    waveform::{AMPLITUDE_PIXELS, POINTS, WaveformFrame},
};

const WAVEFORM_UPDATE: Duration = Duration::from_millis(32);
const WAVEFORM_PEAK_FLOOR: u16 = 1024;
const IMU_UI_UPDATE: Duration = Duration::from_millis(40);
const LOG_REFRESH: Duration = Duration::from_millis(100);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum ViewId {
    Network = 0,
    Imu = 1,
    Microphone = 2,
    Sound = 3,
    Log = 4,
}

impl ViewId {
    pub const ALL: [Self; 5] = [
        Self::Network,
        Self::Imu,
        Self::Microphone,
        Self::Sound,
        Self::Log,
    ];

    pub const fn as_i32(self) -> i32 {
        self as i32
    }

    pub fn from_i32(value: i32) -> Option<Self> {
        Some(match value {
            0 => Self::Network,
            1 => Self::Imu,
            2 => Self::Microphone,
            3 => Self::Sound,
            4 => Self::Log,
            _ => return None,
        })
    }
}

/// CPU0 presentation-sized IMU state. Values are quantized to whole units so
/// Slint property updates remain small and allocation-free.
#[derive(Clone, Copy)]
pub struct ImuDisplay {
    pub roll_deg: i32,
    pub pitch_deg: i32,
    pub yaw_deg: i32,
    pub status: i32,
    pub read_errors: i32,
    pub mag_errors: i32,
    pub mag_status: i32,
    pub mag_field_ut: i32,
    pub mag_calibration: i32,
    pub gyro_bias_ready: bool,
}

struct ImuModel {
    display: ImuDisplay,
    last_revision: u32,
    last_update: Instant,
    dirty: bool,
}

impl ImuModel {
    fn new() -> Self {
        Self {
            display: ImuDisplay {
                roll_deg: 0,
                pitch_deg: 0,
                yaw_deg: 0,
                status: imu::Status::Starting as i32,
                read_errors: 0,
                mag_errors: 0,
                mag_status: imu::MagStatus::Missing as i32,
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
        if now - self.last_update < IMU_UI_UPDATE {
            return;
        }
        self.last_update = now;

        let Some(snapshot) = imu::take_latest() else {
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
            status: snapshot.status as i32,
            read_errors: snapshot.read_errors.min(i32::MAX as u32) as i32,
            mag_errors: snapshot.mag_errors.min(i32::MAX as u32) as i32,
            mag_status: snapshot.mag_status as i32,
            mag_field_ut: round_units(snapshot.mag_field_ut),
            mag_calibration: i32::from(snapshot.mag_calibration_percent),
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
    frame: WaveformFrame,
    samples: [i16; audio::BLOCK_SAMPLES],
    last_sequence: u32,
    last_update: Instant,
    dirty: bool,
}

impl WaveformModel {
    fn new() -> Self {
        Self {
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

        let Some(info) = audio::copy_latest_interleaved(&mut self.samples) else {
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
    ((i32::from(sample) * AMPLITUDE_PIXELS) / scale)
        .clamp(-AMPLITUDE_PIXELS, AMPLITUDE_PIXELS) as i8
}

#[derive(Clone, Copy, Default)]
struct LineRange {
    start: u16,
    end: u16,
}

struct LogData {
    bytes: data_plane::FixedPsramBuffer<u8>,
    lines: data_plane::FixedPsramRing<LineRange, { logger::MAX_LOG_ROWS }>,
    revision: u32,
}

impl LogData {
    fn new() -> Self {
        Self {
            bytes: data_plane::FixedPsramBuffer::filled(logger::HISTORY_BYTES, 0),
            lines: data_plane::FixedPsramRing::new(),
            revision: u32::MAX,
        }
    }

    fn rebuild_lines(&mut self, len: usize) {
        self.lines.clear();

        let bytes = self.bytes.as_slice();
        let mut start = 0usize;

        for end in 0..len {
            if bytes[end] != b'\n' {
                continue;
            }

            if end > start {
                self.lines.push_back(LineRange {
                    start: start as u16,
                    end: end as u16,
                });
            }

            start = end + 1;
        }

        if start < len {
            self.lines.push_back(LineRange {
                start: start as u16,
                end: len as u16,
            });
        }
    }
}

/// Bounded virtualized log model. Its backing byte snapshot and complete line
/// index are preallocated in PSRAM. Only visible ListView rows become
/// SharedStrings in internal RAM.
pub struct LogModel {
    data: RefCell<LogData>,
    notify: ModelNotify,
    last_check: Cell<Instant>,
}

impl LogModel {
    fn new() -> Self {
        Self {
            data: RefCell::new(LogData::new()),
            notify: ModelNotify::default(),
            last_check: Cell::new(Instant::now()),
        }
    }

    fn update_if_due(&self, now: Instant) {
        if now - self.last_check.get() < LOG_REFRESH {
            return;
        }
        self.last_check.set(now);
        self.refresh();
    }

    fn refresh(&self) {
        if logger::revision() == self.data.borrow().revision {
            return;
        }

        let mut data = self.data.borrow_mut();
        let Some((len, revision)) = ({
            logger::snapshot(data.bytes.as_mut_slice())
                .map(|(logs, revision)| (logs.len(), revision))
        }) else {
            return;
        };

        data.rebuild_lines(len);
        data.revision = revision;
        drop(data);

        self.notify.reset();
    }
}

impl Model for LogModel {
    type Data = slint::SharedString;

    fn row_count(&self) -> usize {
        self.data.borrow().lines.len()
    }

    fn row_data(&self, row: usize) -> Option<Self::Data> {
        let data = self.data.borrow();
        let range = *data.lines.get(row)?;
        let text = core::str::from_utf8(
            &data.bytes.as_slice()[usize::from(range.start)..usize::from(range.end)],
        )
        .ok()?;

        Some(slint::SharedString::from(text))
    }

    fn model_tracker(&self) -> &dyn ModelTracker {
        &self.notify
    }
}

/// All mutable application state consumed by the CPU0 presentation layer.
///
/// The model is reference counted only so Slint navigation callbacks can issue
/// commands into it. No Slint component/window object is stored here.
pub struct AppModel {
    active_view: Cell<ViewId>,
    imu: RefCell<ImuModel>,
    waveform: RefCell<WaveformModel>,
    log: Rc<LogModel>,
}

impl AppModel {
    pub fn new() -> Rc<Self> {
        let log = Rc::new(LogModel::new());
        log.refresh();

        Rc::new(Self {
            active_view: Cell::new(ViewId::Log),
            imu: RefCell::new(ImuModel::new()),
            waveform: RefCell::new(WaveformModel::new()),
            log,
        })
    }

    pub fn active_view(&self) -> ViewId {
        self.active_view.get()
    }

    pub fn request_view(&self, raw_view: i32) {
        if let Some(view) = ViewId::from_i32(raw_view) {
            if view != self.active_view.get() {
                self.active_view.set(view);
                match view {
                    ViewId::Imu => self.imu.borrow_mut().mark_dirty(),
                    ViewId::Microphone => self.waveform.borrow_mut().mark_dirty(),
                    _ => {}
                }
            }
        }
    }

    pub fn update(&self, now: Instant) {
        match self.active_view.get() {
            ViewId::Imu => self.imu.borrow_mut().update_if_due(now),
            ViewId::Microphone => self.waveform.borrow_mut().update_if_due(now),
            ViewId::Log => self.log.update_if_due(now),
            _ => {}
        }
    }

    pub fn log_model(&self) -> Rc<LogModel> {
        self.log.clone()
    }

    pub fn take_imu_display(&self) -> Option<ImuDisplay> {
        if self.active_view.get() != ViewId::Imu {
            return None;
        }

        self.imu.borrow_mut().take_display()
    }

    pub fn note_slint_redraw(&self, redrawn: bool) {
        if redrawn && self.active_view.get() == ViewId::Microphone {
            self.waveform.borrow_mut().mark_dirty();
        }
    }

    pub fn take_waveform_frame(&self) -> Option<WaveformFrame> {
        if self.active_view.get() != ViewId::Microphone {
            return None;
        }

        self.waveform.borrow_mut().take_frame()
    }
}
