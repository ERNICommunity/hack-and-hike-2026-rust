//! The green signal: one sample per camera frame, for the last 20 s, and
//! its spectrum, which gives the heart rate.
//!
//! The samples live in a ring buffer in PSRAM. A ring buffer is a fixed
//! array used in a circle: new samples go after the newest one, and when a
//! sample gets too old, the start moves forward. Nothing is ever shifted.
//!
//! The spectrum uses only the last [`HR_WINDOW`]:
//!
//! 1. Resample it onto an even grid ([`RESAMPLE_HZ`]), by linear
//!    interpolation. Frames do not arrive at exactly even times, and a few
//!    may be missing. After this step, that no longer matters.
//! 2. Subtract the best straight line (least squares). This removes the
//!    mean and slow drifts, without touching the 40–180 BPM band.
//! 3. Multiply by a Hann window, so the cut at both ends of the window does
//!    not add false frequencies.
//! 4. A DFT (discrete Fourier transform), only for 40 to 180 BPM in steps of
//!    1 BPM. The strongest frequency is the heart rate.

use core::f32::consts::PI;

use embassy_time::{Duration, Instant};
use hack_and_hike::psram;

/// How long samples are kept.
pub(crate) const HISTORY: Duration = Duration::from_secs(20);
/// Length of the part of the signal that the heart rate comes from, in
/// seconds. The frequency resolution is 1 / 10 s = 6 BPM. 8 s reacts faster,
/// with 7.5 BPM.
const HR_WINDOW_S: u32 = 10;
/// [`HR_WINDOW_S`] as a `Duration`.
pub(crate) const HR_WINDOW: Duration = Duration::from_secs(HR_WINDOW_S as u64);
/// Rate of the evenly spaced samples, in Hz. About the camera's rate.
const RESAMPLE_HZ: u32 = 20;
/// Number of evenly spaced samples in [`HR_WINDOW`].
const HR_SAMPLES: usize = (HR_WINDOW_S * RESAMPLE_HZ) as usize;
/// Lowest heart rate searched for, in beats per minute.
pub(crate) const MIN_BPM: u32 = 40;
/// Highest heart rate searched for, in beats per minute.
pub(crate) const MAX_BPM: u32 = 180;
/// Number of frequencies in a [`Spectrum`]: one per BPM.
pub(crate) const BPM_COUNT: usize = (MAX_BPM - MIN_BPM + 1) as usize;
/// Call `pump` after this many frequencies of the DFT.
const FREQUENCIES_PER_PUMP: usize = 16;
/// Room for 20 s at up to 50 FPS. The camera runs at about 20 FPS, so the
/// time limit, not this size, normally removes old samples.
const CAPACITY: usize = 1000;

/// One measurement: the mean green value of the ROI in one frame.
#[derive(Clone, Copy)]
pub(crate) struct Sample {
    /// When the frame was taken.
    pub(crate) time: Instant,
    /// The mean green value, 0 to 63.
    pub(crate) green: f32,
}

/// The samples of the last [`HISTORY`], oldest first.
pub(crate) struct History {
    /// The ring buffer.
    samples: &'static mut [Sample],
    /// Index of the oldest sample.
    start: usize,
    /// Number of samples stored.
    len: usize,
}

impl History {
    /// An empty history. Call once, before the loop: the memory is never
    /// freed.
    pub(crate) fn new() -> Self {
        let empty = Sample {
            time: Instant::from_ticks(0),
            green: 0.0,
        };
        Self {
            samples: psram::leaked_slice(CAPACITY, empty),
            start: 0,
            len: 0,
        }
    }

    /// Add the newest sample, and remove the samples older than
    /// [`HISTORY`] before it.
    pub(crate) fn push(&mut self, sample: Sample) {
        while let Some(oldest) = self.oldest()
            && sample.time.duration_since(oldest.time) > HISTORY
        {
            self.remove_oldest();
        }
        if self.len == CAPACITY {
            self.remove_oldest();
        }
        let end = (self.start + self.len) % CAPACITY;
        self.samples[end] = sample;
        self.len += 1;
    }

    /// The samples, oldest first.
    pub(crate) fn iter(&self) -> impl Iterator<Item = Sample> + '_ {
        (0..self.len).map(|i| self.samples[(self.start + i) % CAPACITY])
    }

    /// The green signal of the last [`HR_WINDOW`] before `now`, at
    /// [`RESAMPLE_HZ`], by linear interpolation between the samples around
    /// each grid point. `None` when the history does not reach back that
    /// far yet.
    fn resample(&self, now: Instant) -> Option<[f32; HR_SAMPLES]> {
        let start = now.checked_sub(HR_WINDOW)?;
        if self.oldest()?.time > start {
            return None;
        }
        let start_us = start.as_micros();
        let step_us = 1_000_000 / u64::from(RESAMPLE_HZ);

        let mut grid = [0.0; HR_SAMPLES];
        // The next grid point to fill.
        let mut k = 0;
        let mut previous: Option<Sample> = None;
        for sample in self.iter() {
            let time_us = sample.time.as_micros();
            // Fill every grid point up to this sample, between `previous`
            // and this sample.
            while k < HR_SAMPLES {
                let grid_us = start_us + k as u64 * step_us;
                if grid_us > time_us {
                    break;
                }
                grid[k] = match previous {
                    Some(before) if before.time.as_micros() < time_us => {
                        let before_us = before.time.as_micros();
                        let fraction =
                            grid_us.saturating_sub(before_us) as f32 / (time_us - before_us) as f32;
                        before.green + (sample.green - before.green) * fraction
                    }
                    _ => sample.green,
                };
                k += 1;
            }
            previous = Some(sample);
        }
        // Grid points after the newest sample keep its value.
        if let Some(newest) = previous {
            grid[k..].fill(newest.green);
        }
        Some(grid)
    }

    /// A copy of the oldest sample, or `None` when the history is empty.
    fn oldest(&self) -> Option<Sample> {
        (self.len > 0).then(|| self.samples[self.start])
    }

    /// Forget the oldest sample.
    fn remove_oldest(&mut self) {
        self.start = (self.start + 1) % CAPACITY;
        self.len -= 1;
    }
}

/// The spectrum of the last [`HR_WINDOW`], and the heart rate found in it.
pub(crate) struct Spectrum {
    /// Power for each BPM, from [`MIN_BPM`] to [`MAX_BPM`] in steps of 1.
    /// Only the shape matters: the dashboard scales it to its highest value.
    pub(crate) power: [f32; BPM_COUNT],
    /// The heart rate: the BPM with the most power, refined with a
    /// parabola through the peak and its two neighbours.
    pub(crate) bpm: f32,
}

/// The spectrum of the last [`HR_WINDOW`] before `now`. `None` while the
/// history is shorter than that. Calls `pump` every few frequencies, so the
/// caller can empty the camera ring.
pub(crate) fn spectrum(
    history: &History,
    now: Instant,
    mut pump: impl FnMut(),
) -> Option<Spectrum> {
    let mut signal = history.resample(now)?;
    pump();
    remove_trend(&mut signal);
    apply_hann_window(&mut signal);

    let step_s = 1.0 / RESAMPLE_HZ as f32;
    let mut power = [0.0; BPM_COUNT];
    for (index, slot) in power.iter_mut().enumerate() {
        if index % FREQUENCIES_PER_PUMP == 0 {
            pump();
        }
        let bpm = (MIN_BPM as usize + index) as f32;
        let omega = 2.0 * PI * bpm / 60.0;

        // A pointer of length 1 that turns by `omega * step_s` from one
        // sample to the next: (cos, sin) of the angle at that sample. On an
        // even grid, the turn is the same for every sample, so sin and cos
        // are needed only once per frequency, not once per sample.
        let (step_sin, step_cos) = (libm::sinf(omega * step_s), libm::cosf(omega * step_s));
        let (mut cos, mut sin) = (1.0_f32, 0.0_f32);
        let (mut real, mut imaginary) = (0.0_f32, 0.0_f32);
        for &value in &signal {
            real += value * cos;
            imaginary += value * sin;
            (cos, sin) = (
                cos * step_cos - sin * step_sin,
                sin * step_cos + cos * step_sin,
            );
        }
        *slot = real * real + imaginary * imaginary;
    }

    let bpm = peak_bpm(&power);
    Some(Spectrum { power, bpm })
}

/// Subtract the best straight line through the samples (least squares).
fn remove_trend(signal: &mut [f32]) {
    let count = signal.len() as f32;
    let mean_k = (count - 1.0) / 2.0;
    let mean = signal.iter().sum::<f32>() / count;
    let mut covariance = 0.0;
    let mut variance = 0.0;
    for (k, &value) in signal.iter().enumerate() {
        let dk = k as f32 - mean_k;
        covariance += dk * (value - mean);
        variance += dk * dk;
    }
    let slope = covariance / variance;
    for (k, value) in signal.iter_mut().enumerate() {
        *value -= mean + slope * (k as f32 - mean_k);
    }
}

/// Multiply by a Hann window: 0 at both ends, 1 in the middle.
fn apply_hann_window(signal: &mut [f32]) {
    let last = (signal.len() - 1) as f32;
    for (k, value) in signal.iter_mut().enumerate() {
        *value *= 0.5 - 0.5 * libm::cosf(2.0 * PI * k as f32 / last);
    }
}

/// The BPM of the highest power. A parabola through the peak and its two
/// neighbours places it between the 1 BPM steps.
fn peak_bpm(power: &[f32; BPM_COUNT]) -> f32 {
    let mut peak = 0;
    for (index, &value) in power.iter().enumerate() {
        if value > power[peak] {
            peak = index;
        }
    }
    let offset = if peak > 0 && peak < BPM_COUNT - 1 {
        let (left, middle, right) = (power[peak - 1], power[peak], power[peak + 1]);
        let curvature = left - 2.0 * middle + right;
        if curvature < 0.0 {
            0.5 * (left - right) / curvature
        } else {
            0.0
        }
    } else {
        0.0
    };
    MIN_BPM as f32 + peak as f32 + offset
}
