use alloc::rc::Rc;

use embassy_time::{Duration, Instant};
use slint::Model;
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, PointerEventButton, WindowAdapter, WindowEvent};

use crate::{audio, logger, touch};

const SCREEN_WIDTH: u32 = 320;
const SCREEN_HEIGHT: u32 = 240;
const VIEW_MICROPHONE: i32 = 2;
const VIEW_LOG: i32 = 4;

const WAVEFORM_POINTS: usize = 128;
const WAVEFORM_UPDATE: Duration = Duration::from_millis(32);
const WAVEFORM_PEAK_FLOOR: u16 = 1024;
// WaveformPlot is 114 px high: half-height 57 minus the 3 px margin.
const WAVEFORM_AMPLITUDE_PIXELS: i32 = 54;

const LOG_REFRESH: Duration = Duration::from_millis(100);

slint::include_modules!();

struct McuPlatform {
    window: Rc<MinimalSoftwareWindow>,
}

impl Platform for McuPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.window.clone())
    }

    fn duration_since_start(&self) -> core::time::Duration {
        core::time::Duration::from_millis(Instant::now().as_millis())
    }
}

#[derive(Clone, Copy)]
struct TouchState {
    pressed: bool,
    last_point: touch::TouchPoint,
}

impl TouchState {
    const fn new() -> Self {
        Self {
            pressed: false,
            last_point: touch::TouchPoint { x: 0, y: 0 },
        }
    }
}

struct WaveformState {
    left_model: Rc<slint::VecModel<f32>>,
    right_model: Rc<slint::VecModel<f32>>,
    left_pixels: [i8; WAVEFORM_POINTS],
    right_pixels: [i8; WAVEFORM_POINTS],
    samples: [i16; audio::BLOCK_SAMPLES],
    last_sequence: u32,
    last_update: Instant,
}

impl WaveformState {
    fn new(app: &AppWindow) -> Self {
        // Allocate the final vector sizes in one shot instead of repeatedly
        // growing VecModel storage with 128 push() operations per channel.
        let left_model = Rc::new(slint::VecModel::from(alloc::vec![0.0; WAVEFORM_POINTS]));
        let right_model = Rc::new(slint::VecModel::from(alloc::vec![0.0; WAVEFORM_POINTS]));

        app.set_mic_left_samples(left_model.clone().into());
        app.set_mic_right_samples(right_model.clone().into());

        Self {
            left_model,
            right_model,
            left_pixels: [0; WAVEFORM_POINTS],
            right_pixels: [0; WAVEFORM_POINTS],
            samples: [0; audio::BLOCK_SAMPLES],
            last_sequence: 0,
            last_update: Instant::now(),
        }
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

        self.update_models(info);
    }

    fn update_models(&mut self, info: audio::AudioBlockInfo) {
        const FRAMES_PER_POINT: usize = audio::BLOCK_FRAMES / WAVEFORM_POINTS;

        let left_scale = i32::from(info.peak_left.max(WAVEFORM_PEAK_FLOOR));
        let right_scale = i32::from(info.peak_right.max(WAVEFORM_PEAK_FLOOR));

        for point in 0..WAVEFORM_POINTS {
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

            // Slint only needs a model notification if the plotted y pixel
            // actually changed. This suppresses sub-pixel/noise churn.
            if left_pixel != self.left_pixels[point] {
                self.left_pixels[point] = left_pixel;
                self.left_model.set_row_data(
                    point,
                    left_pixel as f32 / WAVEFORM_AMPLITUDE_PIXELS as f32,
                );
            }

            if right_pixel != self.right_pixels[point] {
                self.right_pixels[point] = right_pixel;
                self.right_model.set_row_data(
                    point,
                    right_pixel as f32 / WAVEFORM_AMPLITUDE_PIXELS as f32,
                );
            }
        }
    }
}

fn quantize_waveform(sample: i16, scale: i32) -> i8 {
    ((i32::from(sample) * WAVEFORM_AMPLITUDE_PIXELS) / scale)
        .clamp(-WAVEFORM_AMPLITUDE_PIXELS, WAVEFORM_AMPLITUDE_PIXELS) as i8
}

struct LogState {
    model: Rc<slint::VecModel<slint::SharedString>>,
    snapshot: [u8; logger::SNAPSHOT_BYTES],
    revision: u32,
    last_check: Instant,
}

impl LogState {
    fn new(app: &AppWindow) -> Self {
        let model = Rc::new(slint::VecModel::<slint::SharedString>::default());
        app.set_log_lines(model.clone().into());

        let mut state = Self {
            model,
            snapshot: [0; logger::SNAPSHOT_BYTES],
            revision: u32::MAX,
            last_check: Instant::now(),
        };
        state.refresh();
        state
    }

    fn update_if_due(&mut self, now: Instant) {
        if now - self.last_check < LOG_REFRESH {
            return;
        }
        self.last_check = now;

        if logger::revision() != self.revision {
            self.refresh();
        }
    }

    fn refresh(&mut self) {
        let (logs, revision) = logger::snapshot(&mut self.snapshot);
        sync_log_model(&self.model, logs);
        self.revision = revision;
    }
}

/// Incrementally reconcile the persistent Slint model with an oldest-to-newest
/// logger snapshot. Normal append and ring-buffer eviction only touch changed
/// rows; no complete model/string rebuild is needed.
fn sync_log_model(model: &slint::VecModel<slint::SharedString>, logs: &str) {
    let old_count = model.row_count();
    let new_count = logs.lines().count();
    let max_overlap = old_count.min(new_count);

    let mut overlap = 0usize;

    'candidate: for candidate in (1..=max_overlap).rev() {
        let old_start = old_count - candidate;

        for (offset, line) in logs.lines().take(candidate).enumerate() {
            let Some(old_line) = model.row_data(old_start + offset) else {
                continue 'candidate;
            };

            if old_line.as_str() != line {
                continue 'candidate;
            }
        }

        overlap = candidate;
        break;
    }

    for _ in 0..old_count.saturating_sub(overlap) {
        model.remove(0);
    }

    for line in logs.lines().skip(overlap) {
        model.push(slint::SharedString::from(line));
    }
}

fn logical_position(point: touch::TouchPoint) -> slint::LogicalPosition {
    slint::LogicalPosition {
        x: point.x as f32,
        y: point.y as f32,
    }
}

fn dispatch_move_if_changed(
    window: &MinimalSoftwareWindow,
    state: &mut TouchState,
    point: touch::TouchPoint,
) {
    if point == state.last_point {
        return;
    }

    state.last_point = point;
    window.dispatch_event(WindowEvent::PointerMoved {
        position: logical_position(point),
    });
}

fn dispatch_touch_input(window: &MinimalSoftwareWindow, state: &mut TouchState) {
    while let Some(edge) = touch::try_take_edge() {
        match edge {
            touch::TouchEdge::Pressed(point) => {
                state.pressed = true;
                state.last_point = point;
                window.dispatch_event(WindowEvent::PointerPressed {
                    position: logical_position(point),
                    button: PointerEventButton::Left,
                });
            }
            touch::TouchEdge::Released(point) => {
                if state.pressed {
                    dispatch_move_if_changed(window, state, point);
                }

                window.dispatch_event(WindowEvent::PointerReleased {
                    position: logical_position(point),
                    button: PointerEventButton::Left,
                });
                state.pressed = false;
                state.last_point = point;
            }
        }
    }

    if state.pressed {
        if let Some(point) = touch::take_latest_point() {
            dispatch_move_if_changed(window, state, point);
        }
    }
}

/// CPU0-only presentation state. Slint objects never cross to CPU1.
pub struct Ui {
    window: Rc<MinimalSoftwareWindow>,
    app: AppWindow,
    touch: TouchState,
    waveform: WaveformState,
    logs: LogState,
}

impl Ui {
    pub fn new() -> Self {
        let window = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
        window.set_size(slint::PhysicalSize::new(SCREEN_WIDTH, SCREEN_HEIGHT));

        slint::platform::set_platform(alloc::boxed::Box::new(McuPlatform {
            window: window.clone(),
        }))
        .expect("Failed to initialize Slint platform");

        let app = AppWindow::new().expect("Failed to construct Slint AppWindow");
        app.show().expect("Failed to show Slint AppWindow");

        let waveform = WaveformState::new(&app);
        let logs = LogState::new(&app);

        Self {
            window,
            app,
            touch: TouchState::new(),
            waveform,
            logs,
        }
    }

    /// Prepare one UI frame: consume input, refresh only the active view's
    /// dynamic model, then advance Slint timers/animations.
    pub fn update(&mut self, now: Instant) {
        dispatch_touch_input(&self.window, &mut self.touch);

        match self.app.get_active_view() {
            VIEW_MICROPHONE => self.waveform.update_if_due(now),
            VIEW_LOG => self.logs.update_if_due(now),
            _ => {}
        }

        slint::platform::update_timers_and_animations();
    }

    pub fn window(&self) -> &MinimalSoftwareWindow {
        self.window.as_ref()
    }
}
