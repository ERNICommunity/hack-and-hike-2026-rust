//! Live waveform of both microphone channels.
//!
//! The labels come from the KDL layout; the waveforms are drawn straight to
//! the display so a new block never repaints the whole screen.

mod waveform;

use embassy_time::{Duration, Instant};
use embedded_graphics::primitives::Rectangle;
use hack_and_hike::{
    capabilities::{
        audio::{self, MicBlockInfo, Microphone},
        display::Surface,
    },
    ui::{Canvas, gui, theme},
};
use log::warn;
use static_cell::ConstStaticCell;

use crate::{layout, screens::Screen};

mod generated {
    use embedded_gui::prelude::*;
    embedded_gui::include_gui!("src/bin/demo/screens/microphone/microphone.kdl");
}

const NODES: usize = 16;
const _: () = assert!(generated::MicrophoneApp::WIDTH == layout::CONTENT_SIZE.width);
const _: () = assert!(generated::MicrophoneApp::HEIGHT == layout::CONTENT_SIZE.height);
const UPDATE_PERIOD: Duration = Duration::from_millis(32);
/// Points drawn per channel; each covers `FRAMES_PER_BLOCK / POINTS` frames.
pub(super) const POINTS: usize = 128;
pub(super) const MAX_AMPLITUDE_PIXELS: i32 = 42;
/// Quiet blocks are not stretched to full height beyond this peak.
const PEAK_FLOOR: u16 = 1024;

// One PCM block is 2 KiB: keep it in static memory instead of on the stack.
static SAMPLES: ConstStaticCell<[i16; audio::SAMPLES_PER_BLOCK]> =
    ConstStaticCell::new([0; audio::SAMPLES_PER_BLOCK]);

/// One waveform point per channel, in pixels from the centre line.
#[derive(Clone, Copy)]
pub(super) struct WaveformFrame {
    pub(super) left: [i8; POINTS],
    pub(super) right: [i8; POINTS],
}

pub(crate) struct MicrophoneScreen {
    microphone: Microphone,
    samples: &'static mut [i16; audio::SAMPLES_PER_BLOCK],
    frame: WaveformFrame,
    /// Dropped-block counter of the last block seen; `None` right after
    /// entering, because blocks dropped while another screen was visible do
    /// not count.
    dropped_blocks: Option<u32>,
    last_update: Instant,
    gui: &'static mut gui::Context<NODES>,
    left: Rectangle,
    right: Rectangle,
    labels_dirty: bool,
    frame_dirty: bool,
}

impl MicrophoneScreen {
    pub(crate) fn new(microphone: Microphone) -> Self {
        let gui = gui::context::<NODES>(layout::CONTENT_SIZE.width, layout::CONTENT_SIZE.height);
        let app =
            generated::MicrophoneApp::build(gui).expect("microphone.kdl fits the GUI capacities");
        let left = gui::slot(gui, app.widgets.left_waveform);
        let right = gui::slot(gui, app.widgets.right_waveform);
        for area in [left, right] {
            assert!(
                (area.size.width as usize).is_multiple_of(POINTS),
                "waveform width is a multiple of POINTS"
            );
            assert!(
                MAX_AMPLITUDE_PIXELS < area.size.height as i32 / 2,
                "waveform fits its area"
            );
        }

        Self {
            microphone,
            samples: SAMPLES.take(),
            frame: WaveformFrame {
                left: [0; POINTS],
                right: [0; POINTS],
            },
            dropped_blocks: None,
            last_update: Instant::now(),
            gui,
            left,
            right,
            labels_dirty: true,
            frame_dirty: true,
        }
    }

    /// Reduce the block in `self.samples` to one point per channel and per
    /// `FRAMES_PER_POINT` frames, keeping the loudest sample of each group.
    fn update_frame(&mut self, info: MicBlockInfo) -> bool {
        const FRAMES_PER_POINT: usize = audio::FRAMES_PER_BLOCK / POINTS;
        let left_scale = i32::from(info.peak_left.max(PEAK_FLOOR));
        let right_scale = i32::from(info.peak_right.max(PEAK_FLOOR));
        let mut changed = false;

        let groups = self
            .samples
            .chunks_exact(FRAMES_PER_POINT * audio::CHANNELS);
        for (point, group) in groups.enumerate() {
            let loudest = |channel: usize| {
                group
                    .iter()
                    .skip(channel)
                    .step_by(audio::CHANNELS)
                    .copied()
                    .max_by_key(|sample| sample.unsigned_abs())
                    .unwrap_or(0)
            };
            let left = quantize(loudest(0), left_scale);
            let right = quantize(loudest(1), right_scale);
            changed |= left != self.frame.left[point] || right != self.frame.right[point];
            self.frame.left[point] = left;
            self.frame.right[point] = right;
        }
        changed
    }
}

fn quantize(sample: i16, scale: i32) -> i8 {
    ((i32::from(sample) * MAX_AMPLITUDE_PIXELS) / scale)
        .clamp(-MAX_AMPLITUDE_PIXELS, MAX_AMPLITUDE_PIXELS) as i8
}

impl Screen for MicrophoneScreen {
    fn enter(&mut self) {
        self.labels_dirty = true;
        self.dropped_blocks = None;
    }

    fn update(&mut self, now: Instant) {
        if now - self.last_update < UPDATE_PERIOD {
            return;
        }
        self.last_update = now;

        // Drain the backlog and show only the newest block.
        let mut newest = None;
        while let Some(info) = self.microphone.next_block(self.samples) {
            newest = Some(info);
        }
        let Some(info) = newest else {
            return;
        };

        if let Some(previous) = self.dropped_blocks
            && info.dropped_blocks != previous
        {
            warn!(
                "Microphone dropped blocks while the waveform was active: total {}",
                info.dropped_blocks
            );
        }
        self.dropped_blocks = Some(info.dropped_blocks);
        if self.update_frame(info) {
            self.frame_dirty = true;
        }
    }

    fn present(&mut self, canvas: &mut Canvas, surface: &mut Surface<'_>) {
        if self.labels_dirty {
            self.labels_dirty = false;
            self.frame_dirty = true;
            canvas.clear(theme::WHITE);
            gui::render(self.gui, canvas);
            canvas.show(surface);
        }
        if self.frame_dirty {
            self.frame_dirty = false;
            waveform::render(surface, self.left, &self.frame.left);
            waveform::render(surface, self.right, &self.frame.right);
        }
    }
}
