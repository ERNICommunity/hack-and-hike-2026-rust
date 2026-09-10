//! Fixed-size presentation contract for realtime microphone samples.
//!
//! `WaveformFrame` contains already-quantized vertical pixel offsets. It carries
//! no absolute LCD geometry; placement belongs to `ui::layout`.

pub const POINTS: usize = 128;
pub const MAX_AMPLITUDE_PIXELS: i32 = 42;

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
