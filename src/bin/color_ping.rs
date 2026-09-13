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
use embedded_graphics::{
    pixelcolor::Rgb565,
    prelude::{Point, Size},
    primitives::Rectangle,
};
use serde::{Deserialize, Serialize};

use hack_and_hike::{
    Board,
    capabilities::{
        audio::{self, Speaker},
        display::{Display, SCREEN, SIZE},
        network::{Message, Network},
        touch::{Touch, TouchEvent},
    },
    synth::SineWave,
    ui::{
        Canvas,
        common::{self, Lines},
        theme,
    },
};

esp_bootloader_esp_idf::esp_app_desc!();

const TONE_DURATION_MS: usize = 300;
const TONE_FRAMES: usize = audio::SAMPLE_RATE_HZ as usize * TONE_DURATION_MS / 1_000;
const TONE_VOLUME: f32 = 0.15;
const AUDIO_CHUNK_FRAMES: usize = 128;

/// The explanation at the top; the colour bands fill the rest.
const BANNER: Rectangle = Rectangle::new(Point::zero(), Size::new(SIZE.width, 74));
const BAND_TOP: i32 = BANNER.size.height as i32;
const BAND_SIZE: Size = Size::new(SIZE.width / 4, SIZE.height - BANNER.size.height);
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

    /// The band at this horizontal position.
    fn at_x(x: i32) -> Self {
        let band = (x.max(0) as u32 / BAND_SIZE.width) as usize;
        Self::ALL[band.min(Self::ALL.len() - 1)]
    }

    /// Where the band is drawn.
    fn area(self) -> Rectangle {
        let index = Self::ALL.iter().position(|&c| c == self).unwrap_or(0) as i32;
        Rectangle::new(
            Point::new(index * BAND_SIZE.width as i32, BAND_TOP),
            BAND_SIZE,
        )
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

    const fn frequency_hz(self) -> f32 {
        match self {
            Self::Red => 800.0,
            Self::Green => 1_000.0,
            Self::Blue => 1_200.0,
            Self::Yellow => 1_400.0,
        }
    }
}

/// The message this application sends. The network only moves the bytes;
/// what they mean is up to us.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
struct ColorPing {
    color: Color,
}

impl Message for ColorPing {
    /// Every board in the room hears every message; the name keeps other
    /// applications' messages out of our decoder.
    const NAME: &'static str = "hack-and-hike.color-ping";
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

/// The whole application: the handles it uses and its state.
struct ColorPingApp {
    display: Display,
    touch: Touch,
    network: Network,
    speaker: Speaker,
    canvas: Canvas,
    tone: TonePlayer,
    /// The band under the finger, while one is pressed.
    touched: Option<Color>,
    shown: Shown,
    drawn: Option<Shown>,
}

#[esp_rtos::main]
async fn main(_spawner: Spawner) -> ! {
    let Board {
        display,
        touch,
        network,
        speaker,
        ..
    } = Board::init();

    let mut app = ColorPingApp {
        display,
        touch,
        network,
        speaker,
        canvas: Canvas::new(SIZE),
        tone: TonePlayer::default(),
        touched: None,
        shown: Shown::default(),
        drawn: None,
    };

    loop {
        app.handle_touch();
        app.handle_network();
        app.tone.play(&mut app.speaker);
        app.shown.lit = app.touched.or(app.tone.playing());
        app.draw_if_changed();

        Timer::after(Duration::from_millis(5)).await;
    }
}

impl ColorPingApp {
    /// A tap on a colour band selects that colour and broadcasts it.
    fn handle_touch(&mut self) {
        while let Some(event) = self.touch.next_event() {
            let TouchEvent::Pressed(point) = event else {
                self.touched = None;
                continue;
            };
            if point.y < BAND_TOP {
                continue;
            }

            let color = Color::at_x(point.x);
            self.touched = Some(color);
            self.shown.sent = Some(color);
            if let Err(error) = self.network.broadcast(&ColorPing { color }) {
                log::warn!("The tap was not broadcast: {error}");
            }
        }
    }

    /// A colour from another board starts its tone here.
    fn handle_network(&mut self) {
        while let Some(message) = self.network.next_message() {
            // Other applications' messages arrive here too; they fail to decode.
            if let Ok(ping) = message.decode::<ColorPing>() {
                self.shown.heard = Some(ping.color);
                self.tone.start(ping.color);
            }
        }
    }

    fn draw_if_changed(&mut self) {
        if self.drawn == Some(self.shown) {
            return;
        }
        self.drawn = Some(self.shown);
        draw(&mut self.canvas, self.shown);
        self.canvas.show(&mut self.display.surface(SCREEN));
    }
}

fn draw(canvas: &mut Canvas, shown: Shown) {
    canvas.fill(BANNER, theme::WHITE);
    common::text(
        canvas,
        "COLOR PING",
        Point::new(10, 8),
        common::TITLE_FONT,
        theme::DARK_BLUE,
    );
    let mut lines = Lines::new(canvas, Point::new(10, 27));
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

    for color in Color::ALL {
        let area = color.area();
        let lit = shown.lit == Some(color);
        canvas.fill(area, color.fill(lit));

        let label_area = |y: i32, height: u32| {
            Rectangle::new(
                Point::new(area.top_left.x, y),
                Size::new(area.size.width, height),
            )
        };
        common::centered_text(
            canvas,
            label_area(BAND_TOP + 30, 16),
            color.name(),
            common::TITLE_FONT,
            color.label(lit),
        );
        text.clear();
        let _ = write!(text, "{} Hz", color.frequency_hz() as u32);
        common::centered_text(
            canvas,
            label_area(BAND_TOP + 52, 14),
            &text,
            common::BODY_FONT,
            color.label(lit),
        );
        if shown.sent == Some(color) {
            canvas.fill(
                label_area(SIZE.height as i32 - MARKER_HEIGHT as i32, MARKER_HEIGHT),
                theme::WHITE,
            );
        }
    }
}

/// A tone in progress. Each call to `play` generates as much of it as the
/// speaker queue accepts, so the main loop never blocks on audio.
#[derive(Default)]
struct TonePlayer {
    wave: Option<SineWave>,
    frames_left: usize,
    /// The colour being played, so the screen can light up its band.
    color: Option<Color>,
}

impl TonePlayer {
    /// The colour whose tone is playing, if any.
    const fn playing(&self) -> Option<Color> {
        self.color
    }

    fn start(&mut self, color: Color) {
        self.wave = Some(SineWave::new(color.frequency_hz()));
        self.frames_left = TONE_FRAMES;
        self.color = Some(color);
    }

    fn play(&mut self, speaker: &mut Speaker) {
        let Some(wave) = &mut self.wave else {
            return;
        };
        while self.frames_left > 0 {
            let frames = speaker
                .available_frames()
                .min(AUDIO_CHUNK_FRAMES)
                .min(self.frames_left);
            if frames == 0 {
                return;
            }

            let mut chunk = [0i16; AUDIO_CHUNK_FRAMES * audio::CHANNELS];
            let samples = &mut chunk[..frames * audio::CHANNELS];
            for frame in samples.chunks_exact_mut(audio::CHANNELS) {
                frame.fill(wave.next_sample(TONE_VOLUME));
            }
            speaker.write(samples);
            self.frames_left -= frames;
        }
        self.wave = None;
        self.color = None;
    }
}
