//! Allocation-free CPU0 contract for realtime microphone presentation.
//!
//! The presentation layer owns both the fixed page chrome and waveform pixels.

pub const POINTS: usize = 128;

pub const CANVAS_X: usize = 54;
pub const CANVAS_WIDTH: usize = 256;
pub const CANVAS_HEIGHT: usize = 90;

pub const LEFT_CANVAS_Y: usize = 22;
pub const RIGHT_CANVAS_Y: usize = 140;

pub const CENTER_Y: i32 = (CANVAS_HEIGHT as i32) / 2;
pub const AMPLITUDE_PIXELS: i32 = CENTER_Y - 3;

#[derive(Clone, Copy)]
pub struct WaveformFrame {
    pub left: [i8; POINTS],
    pub right: [i8; POINTS],
}

impl WaveformFrame {
    pub const fn silent() -> Self {
        Self {
            left: [0; POINTS],
            right: [0; POINTS],
        }
    }
}
