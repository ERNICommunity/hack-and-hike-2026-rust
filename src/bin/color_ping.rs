//! Example application combining display, touch, ESP-NOW, and speaker output.
//!
//! Tapping one of four color bands broadcasts that color. A receiving device
//! plays a 300 ms sine tone whose pitch is chosen by the received color.

#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use serde::{Deserialize, Serialize};

use hack_and_hike::{
    Board,
    capabilities::{
        audio::{self, Speaker},
        display::{Display, HEIGHT, Region, WIDTH},
        network::Network,
        touch::{Touch, TouchEdge},
    },
};

esp_bootloader_esp_idf::esp_app_desc!();

const TONE_DURATION_MS: usize = 300;
const TONE_FRAMES: usize = audio::SAMPLE_RATE_HZ as usize * TONE_DURATION_MS / 1_000;
const AUDIO_CHUNK_FRAMES: usize = 128;
const AUDIO_CHUNK_SAMPLES: usize = AUDIO_CHUNK_FRAMES * audio::CHANNELS;

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

/// A tone in progress. Each call to `update` generates as much of it as the
/// speaker queue accepts, so the main loop never blocks on audio.
struct TonePlayer {
    phase: u32,
    phase_step: u32,
    frames_left: usize,
}

impl TonePlayer {
    const fn new() -> Self {
        Self {
            phase: 0,
            phase_step: 0,
            frames_left: 0,
        }
    }

    fn start(&mut self, color: Color) {
        self.phase = 0;
        self.phase_step =
            ((u64::from(color.frequency_hz()) << 32) / u64::from(audio::SAMPLE_RATE_HZ)) as u32;
        self.frames_left = TONE_FRAMES;
    }

    fn update(&mut self, speaker: &mut Speaker) {
        while self.frames_left > 0 {
            let frames = speaker
                .available_frames()
                .min(AUDIO_CHUNK_FRAMES)
                .min(self.frames_left);
            if frames == 0 {
                return;
            }

            let mut chunk = [0i16; AUDIO_CHUNK_SAMPLES];
            let samples = &mut chunk[..frames * audio::CHANNELS];
            for frame in samples.chunks_exact_mut(audio::CHANNELS) {
                frame.fill(sine_sample(self.phase));
                self.phase = self.phase.wrapping_add(self.phase_step);
            }
            speaker.write(samples);
            self.frames_left -= frames;
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

#[esp_rtos::main]
async fn main(_spawner: Spawner) -> ! {
    let Board {
        mut display,
        mut touch,
        mut network,
        mut speaker,
        ..
    } = Board::init();

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
