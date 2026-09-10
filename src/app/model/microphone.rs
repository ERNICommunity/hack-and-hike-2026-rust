//! Microphone presentation model and fixed-size waveform contract.

use embassy_time::{Duration, Instant};

use crate::services::audio;

pub(crate) const POINTS: usize = 128;
pub(crate) const MAX_AMPLITUDE_PIXELS: i32 = 42;

const WAVEFORM_UPDATE: Duration = Duration::from_millis(32);
const WAVEFORM_PEAK_FLOOR: u16 = 1024;

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

pub(super) struct Model {
    input: audio::Input,
    frame: WaveformFrame,
    samples: [i16; audio::BLOCK_SAMPLES],
    last_sequence: u32,
    last_update: Instant,
    dirty: bool,
}

impl Model {
    pub(super) fn new(input: audio::Input) -> Self {
        Self {
            input,
            frame: WaveformFrame::silent(),
            samples: [0; audio::BLOCK_SAMPLES],
            last_sequence: 0,
            last_update: Instant::now(),
            dirty: true,
        }
    }

    pub(super) fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    pub(super) fn update_if_due(&mut self, now: Instant) {
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

    pub(super) fn take_frame(&mut self) -> Option<WaveformFrame> {
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
