//! Display, touch, radio and speaker in one loop.
//!
//! Tapping one of four colour bands broadcasts that colour to every board
//! nearby, and each of them plays a 300 ms tone for it. A band lights up
//! while you hold it, and on the receiving board while its tone plays. Flash
//! this to two boards and tap.

#![no_std]
#![no_main]

use core::fmt::Write as _;

use arrayvec::ArrayString;
use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use embedded_graphics::{pixelcolor::Rgb565, prelude::Point};
use embedded_gui::Rect;
use serde::{Deserialize, Serialize};

use hack_and_hike::{
    Board,
    capabilities::{
        audio::{self, Speaker},
        display::{HEIGHT, Region, WIDTH},
        network::Network,
        touch::{Touch, TouchEdge},
    },
    ui::{
        common::{self, Lines},
        gui::{GuiFramebuffer, GuiSurface},
        theme,
    },
};

esp_bootloader_esp_idf::esp_app_desc!();

const TONE_DURATION_MS: usize = 300;
const TONE_FRAMES: usize = audio::SAMPLE_RATE_HZ as usize * TONE_DURATION_MS / 1_000;
const AUDIO_CHUNK_FRAMES: usize = 128;
const AUDIO_CHUNK_SAMPLES: usize = AUDIO_CHUNK_FRAMES * audio::CHANNELS;

/// The explanation at the top; the colour bands fill the rest.
const BANNER_HEIGHT: u32 = 74;
const BAND_TOP: i32 = BANNER_HEIGHT as i32;
const BAND_HEIGHT: u32 = HEIGHT as u32 - BANNER_HEIGHT;
const BAND_WIDTH: u32 = WIDTH as u32 / 4;
/// The bar marking the colour this board sent last.
const MARKER_HEIGHT: u32 = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum Color {
    Red,
    Green,
    Blue,
    Yellow,
}

impl Color {
    /// Left to right across the screen.
    const ALL: [Self; 4] = [Self::Red, Self::Green, Self::Blue, Self::Yellow];

    /// The colour of the band at this horizontal position.
    fn at_x(x: u16) -> Self {
        let band = usize::from(x) * Self::ALL.len() / WIDTH;
        Self::ALL[band.min(Self::ALL.len() - 1)]
    }

    const fn name(self) -> &'static str {
        match self {
            Self::Red => "RED",
            Self::Green => "GREEN",
            Self::Blue => "BLUE",
            Self::Yellow => "YELLOW",
        }
    }

    /// A band is dim until it is touched or its tone plays.
    const fn fill(self, lit: bool) -> Rgb565 {
        match (self, lit) {
            (Self::Red, false) => Rgb565::new(15, 0, 0),
            (Self::Red, true) => Rgb565::new(31, 0, 0),
            (Self::Green, false) => Rgb565::new(0, 30, 0),
            (Self::Green, true) => Rgb565::new(0, 63, 0),
            (Self::Blue, false) => Rgb565::new(0, 0, 15),
            (Self::Blue, true) => Rgb565::new(0, 0, 31),
            (Self::Yellow, false) => Rgb565::new(15, 30, 0),
            (Self::Yellow, true) => Rgb565::new(31, 63, 0),
        }
    }

    /// Readable text on top of [`Color::fill`].
    const fn label(self, lit: bool) -> Rgb565 {
        match self {
            // The bright halves of these two are too pale for white text.
            Self::Green | Self::Yellow if lit => theme::CHARCOAL,
            _ => theme::WHITE,
        }
    }

    const fn frequency_hz(self) -> u32 {
        match self {
            Self::Red => 800,
            Self::Green => 1_000,
            Self::Blue => 1_200,
            Self::Yellow => 1_400,
        }
    }
}

/// The message this application sends. The network only moves the bytes; what
/// they mean is up to us.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
struct ColorPing {
    color: Color,
}

/// What the screen shows.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
struct Shown {
    /// The last colour this board broadcast.
    sent: Option<Color>,
    /// The last colour another board asked for.
    heard: Option<Color>,
    /// The band that is lit right now: the one under the finger, or the one
    /// whose tone is playing.
    lit: Option<Color>,
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

    let full_screen = Region::new(0, 0, WIDTH, HEIGHT);
    let mut screen = GuiSurface::new(WIDTH, HEIGHT);
    let mut shown = Shown::default();
    let mut touched = None;
    let mut tone = TonePlayer::new();
    let mut drawn = None;

    loop {
        handle_touch(&mut touch, &mut network, &mut shown, &mut touched);
        handle_network(&mut network, &mut tone, &mut shown);
        tone.update(&mut speaker);
        shown.lit = touched.or(tone.playing());

        if drawn != Some(shown) {
            drawn = Some(shown);
            screen.present_custom(&mut display.surface(full_screen), |frame| {
                draw(frame, shown);
            });
        }

        Timer::after(Duration::from_millis(5)).await;
    }
}

/// A tap on a colour band selects that colour and broadcasts it. `touched`
/// holds the band under the finger, so the screen can light it up.
fn handle_touch(
    touch: &mut Touch,
    network: &mut Network,
    shown: &mut Shown,
    touched: &mut Option<Color>,
) {
    while let Some(edge) = touch.next_edge() {
        let TouchEdge::Pressed(point) = edge else {
            *touched = None;
            continue;
        };
        if i32::from(point.y) < BAND_TOP {
            continue;
        }

        let color = Color::at_x(point.x);
        *touched = Some(color);
        shown.sent = Some(color);
        if network.broadcast(&ColorPing { color }).is_err() {
            log::warn!("Send queue is full; the tap was not broadcast");
        }
    }
}

/// A colour from another board starts its tone here.
fn handle_network(network: &mut Network, tone: &mut TonePlayer, shown: &mut Shown) {
    while let Some(message) = network.receive() {
        match message.decode::<ColorPing>() {
            Ok(ping) => {
                shown.heard = Some(ping.color);
                tone.start(ping.color);
            }
            Err(_) => log::warn!("Received a message that is not a ColorPing"),
        }
    }
}

fn draw(frame: &mut GuiFramebuffer, shown: Shown) {
    common::fill(
        frame,
        Rect::new(0, 0, WIDTH as u32, BANNER_HEIGHT),
        theme::WHITE,
    );
    common::text(
        frame,
        "COLOR PING",
        Point::new(10, 8),
        common::TITLE_FONT,
        theme::DARK_BLUE,
    );
    let mut lines = Lines::new(frame, Point::new(10, 27));
    lines.line("Tap a colour: every other board", theme::CHARCOAL);
    lines.line("nearby plays that colour's tone.", theme::CHARCOAL);

    let mut text = ArrayString::<48>::new();
    let _ = write!(
        text,
        "Sent {}   Heard {}",
        shown.sent.map_or("nothing", Color::name),
        shown.heard.map_or("nothing", Color::name)
    );
    lines.line(&text, theme::DARK_GRAY);

    for (band, color) in Color::ALL.into_iter().enumerate() {
        let x = band as i32 * BAND_WIDTH as i32;
        let lit = shown.lit == Some(color);
        common::fill(
            frame,
            Rect::new(x, BAND_TOP, BAND_WIDTH, BAND_HEIGHT),
            color.fill(lit),
        );
        common::centered_text(
            frame,
            Rect::new(x, BAND_TOP + 30, BAND_WIDTH, 16),
            color.name(),
            common::TITLE_FONT,
            color.label(lit),
        );
        text.clear();
        let _ = write!(text, "{} Hz", color.frequency_hz());
        common::centered_text(
            frame,
            Rect::new(x, BAND_TOP + 52, BAND_WIDTH, 14),
            &text,
            common::BODY_FONT,
            color.label(lit),
        );
        if shown.sent == Some(color) {
            common::fill(
                frame,
                Rect::new(
                    x,
                    HEIGHT as i32 - MARKER_HEIGHT as i32,
                    BAND_WIDTH,
                    MARKER_HEIGHT,
                ),
                theme::WHITE,
            );
        }
    }
}

/// A tone in progress. Each call to `update` generates as much of it as the
/// speaker queue accepts, so the main loop never blocks on audio.
struct TonePlayer {
    phase: u32,
    phase_step: u32,
    frames_left: usize,
    /// The colour being played, so the screen can light up its band.
    color: Option<Color>,
}

impl TonePlayer {
    const fn new() -> Self {
        Self {
            phase: 0,
            phase_step: 0,
            frames_left: 0,
            color: None,
        }
    }

    /// The colour whose tone is playing, if any.
    const fn playing(&self) -> Option<Color> {
        self.color
    }

    fn start(&mut self, color: Color) {
        self.phase = 0;
        self.phase_step =
            ((u64::from(color.frequency_hz()) << 32) / u64::from(audio::SAMPLE_RATE_HZ)) as u32;
        self.frames_left = TONE_FRAMES;
        self.color = Some(color);
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
        self.color = None;
    }
}

/// One sine sample for a phase that wraps over the full `u32` range. The top
/// five bits pick one of 32 points of the wave; the volume is kept low.
fn sine_sample(phase: u32) -> i16 {
    const SINE: [i16; 32] = [
        0, 6393, 12539, 18204, 23170, 27245, 30273, 32137, 32767, 32137, 30273, 27245, 23170,
        18204, 12539, 6393, 0, -6393, -12539, -18204, -23170, -27245, -30273, -32137, -32767,
        -32137, -30273, -27245, -23170, -18204, -12539, -6393,
    ];
    const VOLUME_DIVISOR: i16 = 6;

    SINE[(phase >> 27) as usize] / VOLUME_DIVISOR
}
