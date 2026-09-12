//! Microphone waveform application model.

use embassy_time::{Duration, Instant};
use static_cell::ConstStaticCell;

use crate::capabilities::mic;

pub(crate) const POINTS: usize = 128;
pub(crate) const MAX_AMPLITUDE_PIXELS: i32 = 42;

const WAVEFORM_UPDATE: Duration = Duration::from_millis(32);
const WAVEFORM_PEAK_FLOOR: u16 = 1024;

static SAMPLES: ConstStaticCell<[i16; mic::SAMPLES_PER_BLOCK]> =
    ConstStaticCell::new([0; mic::SAMPLES_PER_BLOCK]);

#[derive(Clone, Copy)]
pub(crate) struct WaveformFrame {
    pub(crate) left: [i8; POINTS],
    pub(crate) right: [i8; POINTS],
}

impl WaveformFrame {
    pub(crate) const fn silent() -> Self {
        Self {
            left: [0; POINTS],
            right: [0; POINTS],
        }
    }
}

pub(crate) struct Model {
    microphone: mic::Microphone,
    frame: WaveformFrame,
    samples: &'static mut [i16; mic::SAMPLES_PER_BLOCK],
    last_sequence: u32,
    last_dropped_blocks: u32,
    drop_baseline_pending: bool,
    last_update: Instant,
    dirty: bool,
}

impl Model {
    pub(crate) fn new(microphone: mic::Microphone) -> Self {
        // One full stereo PCM block is 2048 bytes. Keep that persistent scratch
        // storage out of the by-value application/UI construction path. A
        // ConstStaticCell guarantees the zeroed buffer itself is initialized in
        // static storage rather than materialized as a CPU0 stack temporary.
        let samples = SAMPLES.take();

        Self {
            microphone,
            frame: WaveformFrame::silent(),
            samples,
            last_sequence: 0,
            last_dropped_blocks: 0,
            // Capture runs continuously on CPU1 while this view is inactive, but
            // the visualization intentionally does not drain the bounded stream.
            // The first block consumed after activation establishes a new drop
            // baseline; only additional loss while the active consumer is
            // running is actionable.
            drop_baseline_pending: true,
            last_update: Instant::now(),
            dirty: true,
        }
    }

    pub(crate) fn mark_dirty(&mut self) {
        self.dirty = true;
        self.drop_baseline_pending = true;
    }

    pub(crate) fn update_if_due(&mut self, now: Instant) {
        if now - self.last_update < WAVEFORM_UPDATE {
            return;
        }
        self.last_update = now;

        // The capability is a real bounded PCM stream. This visualization keeps
        // the old realtime behavior by draining any backlog and rendering only
        // the newest complete block available for this frame.
        let mut latest = None;
        while let Some(info) = self.microphone.try_read(&mut *self.samples) {
            latest = Some(info);
        }
        let Some(info) = latest else {
            return;
        };

        if self.drop_baseline_pending {
            self.last_dropped_blocks = info.dropped_blocks;
            self.drop_baseline_pending = false;
        } else if info.dropped_blocks != self.last_dropped_blocks {
            ::log::warn!(
                "Microphone PCM queue dropped blocks while active: total={} latest_sequence={}",
                info.dropped_blocks,
                info.sequence
            );
            self.last_dropped_blocks = info.dropped_blocks;
        }
        self.last_sequence = info.sequence;

        if self.update_frame(info) {
            self.dirty = true;
        }
    }

    fn update_frame(&mut self, info: mic::MicBlockInfo) -> bool {
        const FRAMES_PER_POINT: usize = mic::FRAMES_PER_BLOCK / POINTS;
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
                let sample_index = frame * mic::CHANNELS;
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

    pub(crate) fn take_frame(&mut self) -> Option<WaveformFrame> {
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
