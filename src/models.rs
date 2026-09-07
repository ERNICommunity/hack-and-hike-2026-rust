//! CPU0 application models.
//!
//! This module owns state and data refresh policy. It does not own a Slint
//! window/component tree. `ui.rs` is only a presentation adapter around these
//! models.

use alloc::rc::Rc;
use core::cell::{Cell, RefCell};

use embassy_time::{Duration, Instant};

use crate::{
    audio, data_plane, imu, logger, network,
    waveform::{AMPLITUDE_PIXELS, POINTS, WaveformFrame},
};

const WAVEFORM_UPDATE: Duration = Duration::from_millis(32);
const WAVEFORM_PEAK_FLOOR: u16 = 1024;
const IMU_UI_UPDATE: Duration = Duration::from_millis(40);
const NETWORK_UI_UPDATE: Duration = Duration::from_millis(200);
const LOG_REFRESH: Duration = Duration::from_millis(100);
/// Number of complete trailing log lines rendered by the direct MCU log view.
const LOG_VISIBLE_LINES: usize = 23;

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

struct NetworkModel {
    display: Option<network::Snapshot>,
    last_revision: u32,
    last_update: Instant,
    dirty: bool,
}

impl NetworkModel {
    fn new() -> Self {
        Self {
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
        if now - self.last_update < NETWORK_UI_UPDATE {
            return;
        }
        self.last_update = now;

        let Some(snapshot) = network::take_latest() else {
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

/// CPU0 presentation-sized IMU state. Values are quantized to whole units and
/// consumed by the allocation-free direct renderer rather than Slint bindings.
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

/// Return a UTF-8 slice containing at most the newest `line_count` complete
/// logger lines. Logger records always end in `\n`, so newline byte boundaries
/// are also UTF-8 character boundaries.
fn trailing_lines(text: &str, line_count: usize) -> &str {
    if line_count == 0 || text.is_empty() {
        return "";
    }

    let bytes = text.as_bytes();
    let mut seen = 0usize;
    for index in (0..bytes.len()).rev() {
        if bytes[index] != b'\n' {
            continue;
        }

        seen += 1;
        if seen > line_count {
            return &text[index + 1..];
        }
    }

    text
}

/// Fixed-size MCU log presentation model.
///
/// The complete byte snapshot remains in PSRAM. The visible portion is stored
/// only as byte offsets into that fixed buffer, so a logger revision never
/// creates a `SharedString` or any other heap-owned UI value.
struct LogModel {
    bytes: data_plane::FixedPsramBuffer<u8>,
    visible_start: usize,
    visible_len: usize,
    revision: u32,
    last_check: Instant,
    dirty: bool,
}

impl LogModel {
    fn new() -> Self {
        Self {
            bytes: data_plane::FixedPsramBuffer::filled(logger::HISTORY_BYTES, 0),
            visible_start: 0,
            visible_len: 0,
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

        let visible = trailing_lines(logs, LOG_VISIBLE_LINES);
        self.visible_start = visible.as_ptr() as usize - logs.as_ptr() as usize;
        self.visible_len = visible.len();
        self.revision = revision;
        self.dirty = true;
    }

    fn with_text<R>(&mut self, render: impl FnOnce(&str) -> R) -> Option<R> {
        if !self.dirty {
            return None;
        }
        self.dirty = false;

        let end = self.visible_start.saturating_add(self.visible_len);
        let bytes = self.bytes.as_slice().get(self.visible_start..end)?;
        let text = core::str::from_utf8(bytes).ok()?;
        Some(render(text))
    }
}

/// All mutable application state consumed by the CPU0 presentation layer.
///
/// The model is reference counted only so Slint navigation callbacks can issue
/// commands into it. No Slint component/window object is stored here.
pub struct AppModel {
    active_view: Cell<ViewId>,
    network: RefCell<NetworkModel>,
    imu: RefCell<ImuModel>,
    waveform: RefCell<WaveformModel>,
    log: RefCell<LogModel>,
}

impl AppModel {
    pub fn new() -> Rc<Self> {
        let mut log = LogModel::new();
        log.refresh();

        Rc::new(Self {
            active_view: Cell::new(ViewId::Log),
            network: RefCell::new(NetworkModel::new()),
            imu: RefCell::new(ImuModel::new()),
            waveform: RefCell::new(WaveformModel::new()),
            log: RefCell::new(log),
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
                    ViewId::Network => self.network.borrow_mut().mark_dirty(),
                    ViewId::Imu => self.imu.borrow_mut().mark_dirty(),
                    ViewId::Microphone => self.waveform.borrow_mut().mark_dirty(),
                    ViewId::Log => self.log.borrow_mut().mark_dirty(),
                    _ => {}
                }
            }
        }
    }

    pub fn update(&self, now: Instant) {
        match self.active_view.get() {
            ViewId::Network => self.network.borrow_mut().update_if_due(now),
            ViewId::Imu => self.imu.borrow_mut().update_if_due(now),
            ViewId::Microphone => self.waveform.borrow_mut().update_if_due(now),
            ViewId::Log => self.log.borrow_mut().update_if_due(now),
            _ => {}
        }
    }

    pub fn take_network_display(&self) -> Option<network::Snapshot> {
        if self.active_view.get() != ViewId::Network {
            return None;
        }
        self.network.borrow_mut().take_display()
    }

    pub fn with_log_text<R>(&self, render: impl FnOnce(&str) -> R) -> Option<R> {
        if self.active_view.get() != ViewId::Log {
            return None;
        }

        self.log.borrow_mut().with_text(render)
    }

    pub fn take_imu_display(&self) -> Option<ImuDisplay> {
        if self.active_view.get() != ViewId::Imu {
            return None;
        }

        self.imu.borrow_mut().take_display()
    }

    pub fn note_slint_redraw(&self, redrawn: bool) {
        if redrawn {
            match self.active_view.get() {
                ViewId::Network => self.network.borrow_mut().mark_dirty(),
                ViewId::Microphone => self.waveform.borrow_mut().mark_dirty(),
                ViewId::Imu => self.imu.borrow_mut().mark_dirty(),
                ViewId::Log => self.log.borrow_mut().mark_dirty(),
                _ => {}
            }
        }
    }

    pub fn take_waveform_frame(&self) -> Option<WaveformFrame> {
        if self.active_view.get() != ViewId::Microphone {
            return None;
        }

        self.waveform.borrow_mut().take_frame()
    }
}
