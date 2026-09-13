//! Example application combining display, touch, ESP-NOW, and speaker output.
//!
//! Tapping one of four color bands broadcasts that color. A receiving device
//! plays a 300 ms sine tone whose pitch is chosen by the received color.

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use serde::{Deserialize, Serialize};

use crate::{
    capabilities::{
        display::{Display, HEIGHT, Region, WIDTH},
        network::Network,
        speaker::{self, Speaker},
        touch::{Touch, TouchEdge},
    },
    firmware::Bootstrap,
};

const TONE_DURATION_MS: u32 = 300;
const AUDIO_CHUNK_FRAMES: usize = 128;
const AUDIO_CHUNK_SAMPLES: usize = AUDIO_CHUNK_FRAMES * speaker::CHANNELS;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum Color {
    Red,
    Green,
    Blue,
    Yellow,
}

impl Color {
    fn from_x(x: u16) -> Self {
        Self::from_column(usize::from(x))
    }

    fn from_column(x: usize) -> Self {
        match (x * 4 / WIDTH).min(3) {
            0 => Self::Red,
            1 => Self::Green,
            2 => Self::Blue,
            _ => Self::Yellow,
        }
    }

    fn rgb565(self) -> u16 {
        match self {
            Self::Red => 0xF800,
            Self::Green => 0x07E0,
            Self::Blue => 0x001F,
            Self::Yellow => 0xFFE0,
        }
    }

    fn frequency_hz(self) -> u32 {
        match self {
            Self::Red => 800,
            Self::Green => 1_000,
            Self::Blue => 1_200,
            Self::Yellow => 1_400,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
struct ColorPing {
    color: Color,
}

struct TonePlayer {
    phase: u32,
    phase_step: u32,
    frames_left_to_generate: u32,
    pending: [i16; AUDIO_CHUNK_SAMPLES],
    pending_frames: usize,
    pending_offset_frames: usize,
}

impl TonePlayer {
    const fn new() -> Self {
        Self {
            phase: 0,
            phase_step: 0,
            frames_left_to_generate: 0,
            pending: [0; AUDIO_CHUNK_SAMPLES],
            pending_frames: 0,
            pending_offset_frames: 0,
        }
    }

    fn start(&mut self, color: Color) {
        self.phase = 0;
        self.phase_step =
            ((u64::from(color.frequency_hz()) << 32) / u64::from(speaker::SAMPLE_RATE_HZ)) as u32;
        self.frames_left_to_generate = speaker::SAMPLE_RATE_HZ * TONE_DURATION_MS / 1_000;
        self.pending_frames = 0;
        self.pending_offset_frames = 0;
    }

    fn update(&mut self, speaker: &mut Speaker) {
        loop {
            if self.pending_offset_frames < self.pending_frames {
                let first_sample = self.pending_offset_frames * speaker::CHANNELS;
                let last_sample = self.pending_frames * speaker::CHANNELS;
                let written =
                    speaker.try_write_interleaved(&self.pending[first_sample..last_sample]);

                if written == 0 {
                    return;
                }

                self.pending_offset_frames += written;
                if self.pending_offset_frames < self.pending_frames {
                    return;
                }

                self.pending_frames = 0;
                self.pending_offset_frames = 0;
            }

            if self.frames_left_to_generate == 0 {
                return;
            }

            let frames = speaker
                .available_frames()
                .min(AUDIO_CHUNK_FRAMES)
                .min(self.frames_left_to_generate as usize);

            if frames == 0 {
                return;
            }

            let mut phase = self.phase;
            let phase_step = self.phase_step;
            for frame in
                self.pending[..frames * speaker::CHANNELS].chunks_exact_mut(speaker::CHANNELS)
            {
                let sample = sine_sample(phase);
                phase = phase.wrapping_add(phase_step);
                frame.fill(sample);
            }

            self.phase = phase;
            self.frames_left_to_generate -= frames as u32;
            self.pending_frames = frames;
            self.pending_offset_frames = 0;
        }
    }
}

fn sine_sample(phase: u32) -> i16 {
    const SINE: [i16; 32] = [
        0, 6393, 12539, 18204, 23170, 27245, 30273, 32137, 32767, 32137, 30273, 27245, 23170,
        18204, 12539, 6393, 0, -6393, -12539, -18204, -23170, -27245, -30273, -32137, -32767,
        -32137, -30273, -27245, -23170, -18204, -12539, -6393,
    ];

    let index = (phase >> 27) as usize;
    SINE[index] / 6
}

fn draw(display: &mut Display, selected: Color) {
    let full_screen = Region::new(0, 0, WIDTH, HEIGHT);
    let mut surface = display.surface(full_screen);

    surface.render_scanlines(|y, pixels| {
        for (x, pixel) in pixels.iter_mut().enumerate() {
            let color = Color::from_column(x);
            *pixel = color.rgb565();

            if color == selected && y >= HEIGHT - 12 {
                *pixel = 0xFFFF;
            }
        }
    });
}

fn handle_touch(touch: &mut Touch, network: &mut Network, selected: &mut Color, redraw: &mut bool) {
    while let Some(edge) = touch.next_edge() {
        if let TouchEdge::Pressed(point) = edge {
            *selected = Color::from_x(point.x);
            *redraw = true;

            if network.send(None, &ColorPing { color: *selected }).is_err() {
                log::warn!("Color Ping send queue is full");
            }
        }
    }
}

fn handle_network(network: &mut Network, tone: &mut TonePlayer) {
    while let Some(message) = network.receive() {
        match message.decode::<ColorPing>() {
            Ok(ping) => tone.start(ping.color),
            Err(_) => log::warn!("Received a network message with another schema"),
        }
    }
}

pub(crate) async fn run(_spawner: Spawner, bootstrap: Bootstrap) -> ! {
    let Bootstrap {
        mut display,
        mut touch,
        mut network,
        mut speaker,
        ..
    } = bootstrap;

    let mut selected = Color::Red;
    let mut redraw = true;
    let mut tone = TonePlayer::new();

    loop {
        handle_touch(&mut touch, &mut network, &mut selected, &mut redraw);
        handle_network(&mut network, &mut tone);
        tone.update(&mut speaker);

        if redraw {
            draw(&mut display, selected);
            redraw = false;
        }

        Timer::after(Duration::from_millis(5)).await;
    }
}
