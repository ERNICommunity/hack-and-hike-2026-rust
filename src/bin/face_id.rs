//! Face ID: the board learns your face and then tells you apart from
//! everyone else.
//!
//! The left part of the screen shows the camera. A face detector finds
//! your face and draws its box and five landmarks (eyes, nose, mouth
//! corners). The panel on the right says what to do: come closer, move
//! back, look straight. When the face fills the height of the frame and
//! looks at the camera, the recognizer turns it into a 512-number
//! embedding, and the gallery decides who it is.
//!
//! - **ENROLL** starts recording a new person: six embeddings while you
//!   turn your head a little. The people are called `person 1` to
//!   `person 4`.
//! - **DEL** empties the gallery, in flash too.
//! - **-** and **+** change the accept limit: the smallest similarity that
//!   counts as the same person. The panel shows every decision's score
//!   next to the limit.
//!
//! Recognition fuses three embeddings, and a vote over the last decisions
//! keeps the name from flickering. Everything shown on the panel also goes
//! to the log.
//!
//! # How a cycle runs
//!
//! The detector takes a third of a second per frame and the recognizer
//! three quarters of a second per face, and the camera's buffer overflows
//! within a few milliseconds if nobody empties it. So the camera and the
//! screen belong to a task on an interrupt executor (see [`stream`]): it
//! interrupts the networks every few milliseconds to empty the camera's
//! buffer, and shows the live image about ten times per second, with the
//! box of the newest detection drawn on top.
//!
//! A cycle of the main task asks that task for a copy of the newest frame,
//! then works on the copy: the detector on a 4x scaled-down version, the
//! recognizer on the face cut out of a 2x version. The box on the preview
//! is therefore up to one cycle old, while the image is live.
//!
//! # What is kept
//!
//! The gallery is stored in the flash chip when an enrollment completes
//! and when it is emptied, and loaded at start (see [`store`]), so the
//! people survive a restart and a new firmware.

#![no_std]
#![no_main]

extern crate alloc;

use core::{
    cell::Cell,
    fmt::Write as _,
    ops::Range,
    sync::atomic::{AtomicU32, Ordering},
};

use arrayvec::{ArrayString, ArrayVec};
use embassy_executor::Spawner;
use embassy_sync::{
    blocking_mutex::{Mutex, raw::CriticalSectionRawMutex},
    signal::Signal,
};
use embassy_time::{Duration, Instant, Timer};
use embedded_graphics::{
    pixelcolor::{Rgb565, raw::RawU16},
    prelude::*,
    primitives::{PrimitiveStyle, Rectangle},
};
use esp_hal::interrupt::Priority;
use esp_rtos::embassy::InterruptExecutor;
use hack_and_hike::{
    Board,
    capabilities::{
        camera::{self, Camera, Frame},
        display::{BYTES_PER_PIXEL, Display, SCREEN, SIZE, ScanlineSource},
        touch::TouchEvent,
    },
    psram,
    ui::{Canvas, common, theme},
};
use hack_and_hike_vision::{
    align::{CROP_SIZE, align_face, recognizer_input_i8},
    blob::Blob,
    detect::{
        CONTENT_HEIGHT, CONTENT_WIDTH, DEFAULT_NMS_THRESHOLD, DEFAULT_SCORE_THRESHOLD, DOWNSCALE,
        Face, decode, detector_input_i8,
    },
    gallery::{
        Embedding, FUSION_FRAMES, Fusion, Gallery, ImpostorBank, MAX_PEOPLE, Thresholds, Vote,
    },
    gates::{self, Framing, Limits},
    image::{GrayImageMut, Rgb565Frame, RgbImageMut, downscale_to_rgb, rgb_to_gray},
    include_fkb,
    nn::{
        BlobWeights, edgeface,
        lanes::{GeluTable, GroupPlan, NormPlan},
        pack, yunet,
    },
    quality::laplacian_variance,
};
use log::{info, warn};
use static_cell::StaticCell;

esp_bootloader_esp_idf::esp_app_desc!();

/// Which build this is, in the log and on the screen at start-up.
const BUILD_ID: &str = "faceid-9";

/// The recognizer's integer weights, in flash.
static EDGEFACE: &[u8] = include_fkb!("../../assets/models/edgeface_xxs.int8.fkb");
/// The detector's integer weights, in flash.
static YUNET: &[u8] = include_fkb!("../../assets/models/yunet.int8.fkb");
/// Two hundred strangers' embeddings: the impostor bank of the decision.
static IMPOSTORS: &[u8] = include_fkb!("../../assets/models/impostors.fkb");

/// Embeddings recorded per person at enrollment.
const ENROLL_SAMPLES: usize = 6;
/// What to do before each enrollment sample.
const ENROLL_PROMPTS: [&str; ENROLL_SAMPLES] = [
    "look straight",
    "turn a bit left",
    "turn a bit right",
    "chin up a bit",
    "chin down a bit",
    "straight again",
];
/// How much one tap on **-** or **+** changes the accept limit.
const THRESHOLD_STEP: f32 = 0.02;
/// Cycles without a usable face after which the banner goes back to
/// scanning: about a second.
const IDLE_CYCLES_TO_RESET: u8 = 2;
/// The recognizer's scaled-down source: half the camera frame.
const SOURCE_SCALE: usize = 2;
/// Width of the recognizer's source image.
const SOURCE_WIDTH: usize = camera::WIDTH / SOURCE_SCALE;
/// Height of the recognizer's source image.
const SOURCE_HEIGHT: usize = camera::HEIGHT / SOURCE_SCALE;
/// Bytes of one camera row.
const SCANLINE_BYTES: usize = camera::WIDTH * BYTES_PER_PIXEL;
/// Rows copied between two pumps of the camera's buffer.
const COPY_ROWS_PER_PUMP: usize = 8;
/// How often the live preview is drawn at most. Each drawing keeps CPU0
/// busy for about 18 ms (the SPI transfer), time the networks lose.
const PREVIEW_PERIOD: Duration = Duration::from_millis(100);
/// How often the stream task empties the camera's buffer. The buffer holds
/// 40 rows, about 5 ms of the sensor's data.
const PUMP_PERIOD: Duration = Duration::from_millis(2);

// The screen: the camera on the left, the panel on the right.
/// Width of the camera preview, in pixels.
const PREVIEW_WIDTH: usize = 184;
/// The camera preview: the middle columns of the camera image.
const PREVIEW: Rectangle = Rectangle::new(
    Point::zero(),
    Size::new(PREVIEW_WIDTH as u32, camera::HEIGHT as u32),
);
/// The panel with the state, the hints and the buttons.
const PANEL: Rectangle = Rectangle::new(
    Point::new(PREVIEW_WIDTH as i32, 0),
    Size::new(SIZE.width - PREVIEW_WIDTH as u32, SIZE.height),
);
/// Camera columns cut off on the left of the preview. As many are cut off
/// on the right.
const CROP_LEFT: usize = (camera::WIDTH - PREVIEW_WIDTH) / 2;
/// The bytes of each camera row that the preview shows.
const PREVIEW_BYTES: Range<usize> =
    CROP_LEFT * BYTES_PER_PIXEL..(CROP_LEFT + PREVIEW_WIDTH) * BYTES_PER_PIXEL;
/// Half the side of a landmark dot, in pixels.
const DOT: i32 = 2;

const _: () = assert!(camera::WIDTH == SCREEN.size.width as usize);
const _: () = assert!(camera::HEIGHT == SCREEN.size.height as usize);

// The panel, in panel coordinates.
/// Left and right margin of the panel's text.
const MARGIN: i32 = 4;
/// The coloured box with the state.
const BANNER: Rectangle = Rectangle::new(Point::new(MARGIN, 24), Size::new(128, 30));
/// Top of the status lines.
const STATUS_TOP: i32 = 60;
/// The terminal: the newest events, one per line.
const TERMINAL: Rectangle = Rectangle::new(Point::new(MARGIN, 112), Size::new(128, 76));
/// Lines in the terminal.
const TERMINAL_ROWS: usize = 6;
/// Characters per terminal line, including the `> ` prompt.
const TERMINAL_COLUMNS: usize = 21;
/// The button that starts an enrollment.
const ENROLL_BUTTON: Rectangle = Rectangle::new(Point::new(MARGIN, 196), Size::new(58, 40));
/// The button that empties the gallery.
const FORGET_BUTTON: Rectangle = Rectangle::new(Point::new(66, 196), Size::new(34, 40));
/// The button that lowers the limit: stricter.
const MINUS_BUTTON: Rectangle = Rectangle::new(Point::new(104, 196), Size::new(14, 40));
/// The button that raises the limit: more tolerant.
const PLUS_BUTTON: Rectangle = Rectangle::new(Point::new(120, 196), Size::new(14, 40));

// Colours that the theme does not have.
/// The banner and the box when a person is recognized, and when the face
/// passes the gates.
const GREEN: Rgb565 = theme::rgb(0x2E9E4F);
/// The banner when the face is unknown.
const RED: Rgb565 = theme::rgb(0xC0392B);
/// The landmarks.
const YELLOW: Rgb565 = theme::rgb(0xF1C40F);
/// The terminal's background.
const BLACK: Rgb565 = theme::rgb(0x000000);
/// The terminal's text.
const TERMINAL_GREEN: Rgb565 = theme::rgb(0x33FF66);

/// The large buffers, all in PSRAM.
struct Buffers {
    /// One whole camera frame, big-endian RGB565.
    frame: &'static mut [u8],
    /// The frame scaled down 4x, RGB, for the detector.
    small: &'static mut [u8],
    /// The frame scaled down 2x, RGB, for the recognizer's face crop.
    source: &'static mut [u8],
    /// The aligned face, RGB.
    crop: &'static mut [u8],
    /// The aligned face in gray, for the sharpness gate.
    crop_gray: &'static mut [u8],
    /// The detector's input.
    detector_input: &'static mut [i8],
    /// The detector's scratch.
    detector_i16: &'static mut [i16],
    /// The detector's scratch.
    detector_f32: &'static mut [f32],
    /// The recognizer's input.
    recognizer_input: &'static mut [i8],
    /// The recognizer's `i16` scratch.
    recognizer_i16: &'static mut [i16],
    /// The recognizer's `f32` scratch.
    recognizer_f32: &'static mut [f32],
    /// The recognizer's group plans.
    plans: &'static mut [GroupPlan],
    /// The recognizer's LayerNorm plans.
    norm_plans: &'static mut [NormPlan],
    /// The raw embedding.
    embedding: &'static mut [f32],
}

impl Buffers {
    /// Allocate everything.
    fn allocate() -> Self {
        Self {
            frame: psram::leaked_slice(camera::WIDTH * camera::HEIGHT * BYTES_PER_PIXEL, 0),
            small: psram::leaked_slice(CONTENT_WIDTH * CONTENT_HEIGHT * 3, 0),
            source: psram::leaked_slice(SOURCE_WIDTH * SOURCE_HEIGHT * 3, 0),
            crop: psram::leaked_slice(CROP_SIZE * CROP_SIZE * 3, 0),
            crop_gray: psram::leaked_slice(CROP_SIZE * CROP_SIZE, 0),
            detector_input: psram::leaked_slice(yunet::INPUT_SHAPE.len(), 0),
            detector_i16: aligned_psram(yunet::int8::SCRATCH_I16_LEN, 0),
            detector_f32: aligned_psram(yunet::int8::F32_SCRATCH_LEN, 0.0),
            recognizer_input: psram::leaked_slice(CROP_SIZE * CROP_SIZE * 3, 0),
            recognizer_i16: aligned_psram(edgeface::int8::SCRATCH_I16_LEN, 0),
            recognizer_f32: aligned_psram(edgeface::int8::SCRATCH_F32_LEN, 0.0),
            plans: psram::leaked_slice(edgeface::int8::SCRATCH_PLANS, GroupPlan::ZERO),
            norm_plans: psram::leaked_slice(
                edgeface::int8::SCRATCH_NORM_PLANS,
                NormPlan::new(&[1.0; 8], &[0.0; 8], &[1.0; 8]),
            ),
            embedding: psram::leaked_slice(edgeface::EMBEDDING_LEN, 0.0),
        }
    }
}

/// A PSRAM slice of `len` values that starts on a 16-byte boundary, as
/// the vector unit's loads need.
/// A copy of a weights file in PSRAM with its linear and convolution
/// weights grouped by eight output channels, the layout the vector unit
/// reads (`nn::pack`); PSRAM also delivers about four times flash's
/// bandwidth. Built one tensor at a time with a pause after each, so the
/// tasks on CPU1, which run from the same flash, keep their share of it:
/// a continuous copy starves them.
async fn packed_copy(file: &'static [u8]) -> &'static [u8] {
    let source = Blob::parse(file).expect("a valid weights file");
    let copy = aligned_psram::<u8>(pack::packed_len(&source), 0);
    let start = Instant::now();
    pack::pack_header(&source, copy);
    for index in 0..source.len() {
        pack::pack_entry(&source, index, copy);
        Timer::after(Duration::from_millis(2)).await;
    }
    info!(
        "packed {} KiB of weights into PSRAM in {} ms",
        copy.len() / 1024,
        start.elapsed().as_millis()
    );
    copy
}

fn aligned_psram<T: Clone + 'static>(len: usize, value: T) -> &'static mut [T] {
    let spare = 16 / core::mem::size_of::<T>().max(1);
    let raw = psram::leaked_slice::<T>(len + spare, value);
    let skip = raw.as_ptr().align_offset(16);
    &mut raw[skip..skip + len]
}

#[esp_rtos::main]
async fn main(_spawner: Spawner) -> ! {
    let Board {
        mut display,
        mut touch,
        camera,
        spare_interrupt,
        ..
    } = Board::init();
    info!("=== face id [{BUILD_ID}] ===");

    let Some(camera) = camera else {
        warn!("No camera found; face id needs it");
        let mut canvas = Canvas::new(SIZE);
        canvas.clear(theme::WHITE);
        common::centered_text(
            &mut canvas,
            SCREEN,
            "No camera found",
            common::TITLE_FONT,
            theme::CHARCOAL,
        );
        canvas.show(&mut display.surface(SCREEN));
        loop {
            Timer::after(Duration::from_secs(1)).await;
        }
    };

    let mut buffers = Buffers::allocate();
    let gallery = psram::leaked_value(Gallery::new);
    let mut store = store::Store::open();
    match store.load(gallery) {
        Ok(0) => info!("flash: no enrollments stored"),
        Ok(people) => info!("flash: {people} people loaded"),
        Err(error) => warn!("flash: could not read the enrollments: {error:?}"),
    }
    let recognizer = BlobWeights::new(packed_copy(EDGEFACE).await)
        .expect("the packed recognizer weights are a valid file");
    let detector = BlobWeights::new(packed_copy(YUNET).await)
        .expect("the packed detector weights are a valid file");
    let impostors = Blob::parse(IMPOSTORS).expect("the impostor bank is a valid file");
    let impostors = impostors.get("impostors").expect("the impostors tensor");
    let bank = ImpostorBank::from_i8(impostors.i8_slice(), impostors.scale);
    // The GELU table (64 KB), built once in internal RAM: its reads are
    // random, and in PSRAM half of them would miss the cache.
    let gelu = GeluTable::build(alloc::vec![0i16; GeluTable::LEN].leak());
    hack_and_hike::logging::report_memory("face id ready");

    let mut app = App {
        thresholds: Thresholds::DEFAULT,
        limits: Limits::DEFAULT,
        mode: Mode::Scanning,
        fusion: Fusion::new(),
        vote: Vote::new(),
        shown_name: None,
        last_score: None,
        hint: Hint::NoFace,
        timing: Timing::default(),
        idle_cycles: 0,
        terminal: Terminal::new(),
    };
    app.terminal.say_fmt(format_args!("face id {BUILD_ID}"));
    app.terminal.say("tap ENROLL to start");

    let mut canvas: &'static mut Canvas = psram::leaked_value(|| Canvas::new(PANEL.size));
    let mut shown = None;

    // From here on, the camera and the screen belong to the stream task.
    static EXECUTOR: StaticCell<InterruptExecutor<2>> = StaticCell::new();
    let executor = EXECUTOR.init(InterruptExecutor::new(spare_interrupt));
    executor
        .start(Priority::Priority1)
        .spawn(stream(camera, display).expect("the stream task starts once"));

    loop {
        while let Some(event) = touch.next_event() {
            if let TouchEvent::Pressed(point) = event {
                app.on_press(point - PANEL.top_left, gallery, &mut store);
            }
        }

        // 1. A copy of the newest frame, from the stream task.
        let cycle_started = Instant::now();
        let previews_before = PREVIEWS_SHOWN.load(Ordering::Relaxed);
        FRAME_WANTED.signal(core::mem::take(&mut buffers.frame));
        buffers.frame = FRAME_COPIED.wait().await;
        app.timing.capture_ms = cycle_started.elapsed().as_millis() as u32;

        // 2. Detect.
        let started = Instant::now();
        let face = detect(&detector, &mut buffers);
        app.timing.detect_ms = started.elapsed().as_millis() as u32;
        let judged = face.map(|face| app.judge(&face));

        // 3. The box and the landmarks on the live preview.
        OVERLAY.lock(|overlay| {
            overlay.set(
                judged
                    .as_ref()
                    .map(|judged| (judged.face, app.box_color(judged))),
            );
        });
        app.hint = judged.as_ref().map_or(Hint::NoFace, |judged| judged.hint);

        // 4. Recognize or record, when the face passes the gates and there
        // is a reason to.
        let mut embedded = false;
        if let Some(judged) = &judged
            && judged.hint == Hint::Good
            && (app.mode.is_enrolling() || !gallery.is_empty())
        {
            let started = Instant::now();
            let embedding = embed(&recognizer, &gelu, &judged.face, &mut buffers, &app.limits);
            app.timing.embed_ms = started.elapsed().as_millis() as u32;
            match embedding {
                Some(embedding) => {
                    embedded = true;
                    app.on_embedding(embedding, gallery, &bank, &mut store);
                }
                None => app.hint = Hint::Blurred,
            }
        }
        if embedded {
            app.idle_cycles = 0;
        } else if !app.mode.is_enrolling() {
            // No embedding this cycle: the fusion must not mix faces from
            // long ago with new ones, and after a moment without a face
            // the banner goes back to scanning.
            app.fusion.clear();
            app.idle_cycles = app.idle_cycles.saturating_add(1);
            if app.idle_cycles == IDLE_CYCLES_TO_RESET && app.shown_name.is_some() {
                app.vote.clear();
                app.shown_name = None;
                app.last_score = None;
                app.terminal.say("face gone");
            }
        }
        let cycle_ms = cycle_started.elapsed().as_millis().max(1) as u32;
        let previews = PREVIEWS_SHOWN
            .load(Ordering::Relaxed)
            .wrapping_sub(previews_before);
        info!(
            "cycle: capture {} ms, detect {} ms, embed {} ms, preview {:.1} fps, {}, {}",
            app.timing.capture_ms,
            app.timing.detect_ms,
            if embedded { app.timing.embed_ms } else { 0 },
            previews as f32 * 1000.0 / cycle_ms as f32,
            match &judged {
                Some(judged) => judged.describe(),
                None => ArrayString::from("no face").expect("fits"),
            },
            app.hint.text()
        );

        // 5. The panel, when something changed.
        let status = app.status(gallery);
        if shown != Some(status) {
            shown = Some(status);
            app.draw_panel(canvas, &status, gallery);
            PANEL_WANTED.signal(canvas);
            canvas = PANEL_SHOWN.wait().await;
        }
        embassy_futures::yield_now().await;
    }
}

/// The main task's empty frame buffer, for the stream task to fill.
static FRAME_WANTED: Signal<CriticalSectionRawMutex, &'static mut [u8]> = Signal::new();
/// The same buffer back, holding the newest camera frame.
static FRAME_COPIED: Signal<CriticalSectionRawMutex, &'static mut [u8]> = Signal::new();
/// The drawn panel, for the stream task to show.
static PANEL_WANTED: Signal<CriticalSectionRawMutex, &'static mut Canvas> = Signal::new();
/// The same canvas back, once it is on the screen.
static PANEL_SHOWN: Signal<CriticalSectionRawMutex, &'static mut Canvas> = Signal::new();
/// The newest detection, in frame pixels, and the colour of its box.
static OVERLAY: Mutex<CriticalSectionRawMutex, Cell<Option<(Face, Rgb565)>>> =
    Mutex::new(Cell::new(None));
/// Preview frames drawn so far, for the frame rate in the log.
static PREVIEWS_SHOWN: AtomicU32 = AtomicU32::new(0);

/// The camera and the screen, on an interrupt executor of CPU0.
///
/// The main task runs the networks for hundreds of milliseconds without a
/// pause. This task interrupts it every [`PUMP_PERIOD`] to empty the
/// camera's buffer, so the camera never stops, and draws the newest frame
/// with the [`OVERLAY`] every [`PREVIEW_PERIOD`]. It also serves the main
/// task: it copies the newest frame into [`FRAME_WANTED`]'s buffer and
/// shows [`PANEL_WANTED`]'s canvas.
///
/// Nothing here waits for the sensor, except the start of the capture after
/// start-up or a dropped frame (at most two frame periods).
#[embassy_executor::task]
async fn stream(mut camera: Camera, mut display: Display) -> ! {
    let mut next_preview = Instant::now();
    // Whether the newest frame is not drawn yet.
    let mut fresh = false;
    let mut frame_target = None;
    loop {
        camera.pump();
        // Move on to the newest whole frame, without waiting for one. After
        // an overflow of the camera's buffer, `finish` ends the stopped
        // capture, and the next `begin_frame` starts it again.
        if let Some(frame) = camera.begin_frame()
            && frame.can_finish()
        {
            frame.finish();
            fresh = true;
        }
        if frame_target.is_none() {
            frame_target = FRAME_WANTED.try_take();
        }
        if let Some(mut frame) = camera.begin_frame() {
            if let Some(target) = frame_target.take() {
                copy_frame(&mut frame, target);
                FRAME_COPIED.signal(target);
            }
            if fresh && Instant::now() >= next_preview {
                fresh = false;
                next_preview = Instant::now() + PREVIEW_PERIOD;
                let face = OVERLAY.lock(Cell::get);
                display
                    .surface(PREVIEW)
                    .render_from(&mut Preview { frame, face });
                PREVIEWS_SHOWN.fetch_add(1, Ordering::Relaxed);
            }
        }
        if let Some(canvas) = PANEL_WANTED.try_take() {
            canvas.show_while(&mut display.surface(PANEL), || camera.pump());
            PANEL_SHOWN.signal(canvas);
        }
        Timer::after(PUMP_PERIOD).await;
    }
}

/// Copy the frame into `target`, emptying the camera's buffer every few
/// rows so that the capture of the next frame does not overflow.
fn copy_frame(frame: &mut Frame<'_>, target: &mut [u8]) {
    for y in 0..camera::HEIGHT {
        target[y * SCANLINE_BYTES..(y + 1) * SCANLINE_BYTES].copy_from_slice(frame.scanline(y));
        if y % COPY_ROWS_PER_PUMP == COPY_ROWS_PER_PUMP - 1 {
            frame.pump();
        }
    }
}

/// Run the detector on the copied frame: the best face, in frame pixels.
fn detect(weights: &BlobWeights<'_>, buffers: &mut Buffers) -> Option<Face> {
    let source = Rgb565Frame::new(buffers.frame, camera::WIDTH, camera::HEIGHT);
    let mut small = RgbImageMut::new(buffers.small, CONTENT_WIDTH, CONTENT_HEIGHT);
    downscale_to_rgb(&source, DOWNSCALE, &mut small);
    detector_input_i8(&small.as_image(), buffers.detector_input);
    let heads = yunet::int8::forward(
        weights,
        buffers.detector_input,
        yunet::int8::Scratch::new(buffers.detector_i16, buffers.detector_f32),
    );
    let faces = decode(&heads, DEFAULT_SCORE_THRESHOLD, DEFAULT_NMS_THRESHOLD);
    faces.best().map(|face| face.scaled(DOWNSCALE as f32))
}

/// Cut the face out of the copied frame, check its sharpness and run the
/// recognizer. `None` when the crop is blurred or the landmarks are
/// degenerate.
fn embed(
    weights: &BlobWeights<'_>,
    gelu: &GeluTable<'_>,
    face: &Face,
    buffers: &mut Buffers,
    limits: &Limits,
) -> Option<Embedding> {
    let source = Rgb565Frame::new(buffers.frame, camera::WIDTH, camera::HEIGHT);
    let mut half = RgbImageMut::new(buffers.source, SOURCE_WIDTH, SOURCE_HEIGHT);
    downscale_to_rgb(&source, SOURCE_SCALE, &mut half);
    let landmarks = face
        .landmarks
        .map(|[x, y]| [x / SOURCE_SCALE as f32, y / SOURCE_SCALE as f32]);
    let mut crop = RgbImageMut::new(buffers.crop, CROP_SIZE, CROP_SIZE);
    align_face(&landmarks, &half.as_image(), &mut crop)?;
    let mut gray = GrayImageMut::new(buffers.crop_gray, CROP_SIZE, CROP_SIZE);
    rgb_to_gray(&crop.as_image(), &mut gray);
    let sharpness = laplacian_variance(&gray.as_image());
    if sharpness < limits.min_sharpness {
        info!("crop too blurred: sharpness {sharpness:.0}");
        return None;
    }
    recognizer_input_i8(&crop.as_image(), buffers.recognizer_input);
    edgeface::int8::forward(
        weights,
        gelu,
        buffers.recognizer_input,
        edgeface::int8::Scratch::new(
            buffers.recognizer_i16,
            buffers.recognizer_f32,
            buffers.plans,
            buffers.norm_plans,
        ),
        buffers.embedding,
    );
    Some(Embedding::from_raw(buffers.embedding))
}

/// What the app is doing.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Looking for known faces.
    Scanning,
    /// Recording a new person.
    Enrolling {
        /// The person's slot in the gallery.
        index: usize,
        /// Samples recorded so far.
        samples: usize,
    },
}

impl Mode {
    /// Whether a person is being recorded.
    fn is_enrolling(self) -> bool {
        matches!(self, Self::Enrolling { .. })
    }
}

/// What the user should do, from the last detection.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Hint {
    /// No face in the frame.
    NoFace,
    /// The face is too small.
    Closer,
    /// The face is too large.
    Back,
    /// The face crosses an edge.
    Centre,
    /// The face is turned away.
    LookStraight,
    /// The crop was not sharp enough.
    Blurred,
    /// The face passed every gate.
    Good,
}

impl Hint {
    /// The text on the panel.
    fn text(self) -> &'static str {
        match self {
            Self::NoFace => "no face",
            Self::Closer => "come closer",
            Self::Back => "move back",
            Self::Centre => "centre your face",
            Self::LookStraight => "look straight",
            Self::Blurred => "hold still",
            Self::Good => "face ok",
        }
    }
}

/// A detected face with the gates' verdict.
struct Judged {
    /// The face, in frame pixels.
    face: Face,
    /// The verdict.
    hint: Hint,
}

impl Judged {
    /// One line for the log.
    fn describe(&self) -> ArrayString<64> {
        let mut text = ArrayString::new();
        let _ = write!(
            text,
            "face {:.2} at ({:.0},{:.0}) {:.0}x{:.0}",
            self.face.score, self.face.x, self.face.y, self.face.width, self.face.height
        );
        text
    }
}

/// How long the steps of the last cycle took.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct Timing {
    /// Waiting for and copying the frame.
    capture_ms: u32,
    /// The detector.
    detect_ms: u32,
    /// The recognizer, of the last cycle that ran it.
    embed_ms: u32,
}

/// The application's state.
struct App {
    /// The decision's limits.
    thresholds: Thresholds,
    /// The gates' limits.
    limits: Limits,
    /// What the app is doing.
    mode: Mode,
    /// The last embeddings of the face in front of the camera.
    fusion: Fusion,
    /// The last decisions.
    vote: Vote,
    /// The person the vote settled on: their gallery index, or `None`
    /// for unknown; `None` as well before any decision.
    shown_name: Option<Option<u8>>,
    /// The score of the last decision, in hundredths.
    last_score: Option<i16>,
    /// What the user should do.
    hint: Hint,
    /// The last cycle's timing.
    timing: Timing,
    /// Cycles in a row without a usable face.
    idle_cycles: u8,
    /// The newest events.
    terminal: Terminal,
}

impl App {
    /// Handle a tap at `point`, in panel coordinates.
    fn on_press(&mut self, point: Point, gallery: &mut Gallery, store: &mut store::Store) {
        if ENROLL_BUTTON.contains(point) {
            self.start_enrolling(gallery);
        } else if FORGET_BUTTON.contains(point) {
            let names: ArrayVec<ArrayString<16>, MAX_PEOPLE> = gallery
                .people()
                .iter()
                .map(|person| ArrayString::from(person.name()).expect("names fit"))
                .collect();
            for name in &names {
                gallery.forget(name);
            }
            self.mode = Mode::Scanning;
            self.fusion.clear();
            self.vote.clear();
            self.shown_name = None;
            self.last_score = None;
            match store.save(gallery) {
                Ok(()) => self.terminal.say("gallery emptied, flash too"),
                Err(error) => {
                    warn!("flash: could not save: {error:?}");
                    self.terminal.say("gallery emptied (flash failed)");
                }
            }
        } else if MINUS_BUTTON.contains(point) {
            self.thresholds.accept = (self.thresholds.accept - THRESHOLD_STEP).max(0.0);
            self.terminal
                .say_fmt(format_args!("limit {:.2}", self.thresholds.accept));
        } else if PLUS_BUTTON.contains(point) {
            self.thresholds.accept = (self.thresholds.accept + THRESHOLD_STEP).min(1.0);
            self.terminal
                .say_fmt(format_args!("limit {:.2}", self.thresholds.accept));
        }
    }

    /// Add a person and start recording them.
    fn start_enrolling(&mut self, gallery: &mut Gallery) {
        if self.mode.is_enrolling() {
            self.terminal.say("already enrolling");
            return;
        }
        // The first free name.
        let mut name = ArrayString::<16>::new();
        let mut index = None;
        for number in 1..=MAX_PEOPLE {
            name.clear();
            let _ = write!(name, "person {number}");
            if gallery.person(&name).is_none() {
                if gallery.enroll(&name).is_some() {
                    index = Some(gallery.len() - 1);
                }
                break;
            }
        }
        let Some(index) = index else {
            self.terminal.say("gallery full: FORGET");
            return;
        };
        self.mode = Mode::Enrolling { index, samples: 0 };
        self.fusion.clear();
        self.vote.clear();
        self.shown_name = None;
        self.terminal.say_fmt(format_args!("enrolling {name}"));
        self.terminal.say(ENROLL_PROMPTS[0]);
    }

    /// The gates' verdict on `face`.
    fn judge(&self, face: &Face) -> Judged {
        let framing = gates::framing(
            face,
            camera::WIDTH as f32,
            camera::HEIGHT as f32,
            &self.limits,
        );
        let hint = match framing {
            Framing::TooFar => Hint::Closer,
            Framing::TooClose => Hint::Back,
            Framing::OffCentre => Hint::Centre,
            Framing::Good => {
                if gates::pose(&face.landmarks).is_frontal(&self.limits) {
                    Hint::Good
                } else {
                    Hint::LookStraight
                }
            }
        };
        Judged { face: *face, hint }
    }

    /// The colour of the face box.
    fn box_color(&self, judged: &Judged) -> Rgb565 {
        match (judged.hint, self.mode) {
            (Hint::Good, Mode::Enrolling { .. }) => theme::LIGHT_BLUE,
            (Hint::Good, Mode::Scanning) => GREEN,
            _ => theme::WHITE,
        }
    }

    /// A new embedding of the face in front of the camera: a sample while
    /// enrolling, otherwise a probe.
    fn on_embedding(
        &mut self,
        embedding: Embedding,
        gallery: &mut Gallery,
        bank: &ImpostorBank<'_>,
        store: &mut store::Store,
    ) {
        if let Mode::Enrolling { index, samples } = self.mode {
            let Some(person) = gallery
                .people()
                .get(index)
                .map(|person| ArrayString::<16>::from(person.name()).expect("names fit"))
            else {
                self.mode = Mode::Scanning;
                return;
            };
            let Some(person) = gallery.person_mut(&person) else {
                self.mode = Mode::Scanning;
                return;
            };
            if !person.add_template(embedding) {
                self.terminal.say("person full");
                self.mode = Mode::Scanning;
                return;
            }
            let samples = samples + 1;
            self.terminal
                .say_fmt(format_args!("sample {samples}/{ENROLL_SAMPLES} kept"));
            if samples >= ENROLL_SAMPLES {
                self.terminal
                    .say_fmt(format_args!("{} enrolled", person.name()));
                self.mode = Mode::Scanning;
                self.vote.clear();
                self.shown_name = None;
                match store.save(gallery) {
                    Ok(()) => self.terminal.say("saved to flash"),
                    Err(error) => {
                        warn!("flash: could not save: {error:?}");
                        self.terminal.say("flash save failed");
                    }
                }
            } else {
                self.mode = Mode::Enrolling { index, samples };
                self.terminal.say(ENROLL_PROMPTS[samples]);
            }
            return;
        }

        self.fusion.push(embedding);
        let Some(fused) = self.fusion.fused() else {
            return;
        };
        let verdict = gallery.match_probe(&fused, bank, &self.thresholds);
        self.last_score = Some((verdict.score() * 100.0) as i16);
        let decision = verdict.name().and_then(|name| {
            gallery
                .people()
                .iter()
                .position(|person| person.name() == name)
                .map(|index| index as u8)
        });
        info!(
            "decision: {} score {:.2} (limit {:.2}), fused over {FUSION_FRAMES} frames",
            verdict.name().unwrap_or("unknown"),
            verdict.score(),
            self.thresholds.accept
        );
        if let Some(settled) = self.vote.push(decision) {
            self.shown_name = Some(settled);
            match settled.and_then(|index| gallery.people().get(usize::from(index))) {
                Some(person) => self
                    .terminal
                    .say_fmt(format_args!("hello {}", person.name())),
                None => self.terminal.say("unknown face"),
            }
        }
    }

    /// Everything the panel shows.
    fn status(&self, gallery: &Gallery) -> Status {
        Status {
            mode: self.mode,
            people: gallery.len() as u8,
            shown_name: self.shown_name,
            last_score: self.last_score,
            accept: (self.thresholds.accept * 100.0) as i16,
            hint: self.hint,
            timing: self.timing,
            terminal: self.terminal.revision,
        }
    }

    /// Draw the whole panel onto `canvas`.
    fn draw_panel(&self, canvas: &mut Canvas, status: &Status, gallery: &Gallery) {
        canvas.clear(theme::CHARCOAL);
        common::text(
            canvas,
            "FACE ID",
            Point::new(MARGIN, 6),
            common::TITLE_FONT,
            theme::WHITE,
        );

        let mut label = ArrayString::<24>::new();
        let color = match (status.mode, status.shown_name) {
            (Mode::Enrolling { samples, .. }, _) => {
                let _ = write!(label, "ENROLL {samples}/{ENROLL_SAMPLES}");
                theme::LIGHT_BLUE
            }
            (Mode::Scanning, _) if status.people == 0 => {
                label.push_str("NOBODY ENROLLED");
                theme::DARK_GRAY
            }
            (Mode::Scanning, Some(Some(index))) => {
                let name = gallery
                    .people()
                    .get(usize::from(index))
                    .map_or("?", |person| person.name());
                let _ = write!(label, "{}", name.to_ascii_uppercase_array());
                GREEN
            }
            (Mode::Scanning, Some(None)) => {
                label.push_str("UNKNOWN");
                RED
            }
            (Mode::Scanning, None) => {
                label.push_str("SCANNING");
                theme::DARK_GRAY
            }
        };
        let Ok(()) = BANNER
            .into_styled(PrimitiveStyle::with_fill(color))
            .draw(canvas);
        common::centered_text(canvas, BANNER, &label, common::TITLE_FONT, theme::WHITE);

        let mut line = ArrayString::<TERMINAL_COLUMNS>::new();
        let _ = match status.last_score {
            Some(score) => write!(line, "score {} ", Fixed(score)),
            None => write!(line, "score --   "),
        };
        let _ = write!(line, "lim {}", Fixed(status.accept));
        common::text(
            canvas,
            &line,
            Point::new(MARGIN, STATUS_TOP),
            common::DENSE_FONT,
            theme::WHITE,
        );
        common::text(
            canvas,
            status.hint.text(),
            Point::new(MARGIN, STATUS_TOP + common::DENSE_LINE_HEIGHT),
            common::DENSE_FONT,
            if status.hint == Hint::Good {
                GREEN
            } else {
                theme::LIGHT_GRAY
            },
        );
        line.clear();
        let _ = write!(
            line,
            "det {}ms rec {}ms",
            status.timing.detect_ms, status.timing.embed_ms
        );
        common::text(
            canvas,
            &line,
            Point::new(MARGIN, STATUS_TOP + 2 * common::DENSE_LINE_HEIGHT),
            common::DENSE_FONT,
            theme::LIGHT_GRAY,
        );
        line.clear();
        let _ = write!(line, "{} of {MAX_PEOPLE} enrolled", status.people);
        common::text(
            canvas,
            &line,
            Point::new(MARGIN, STATUS_TOP + 3 * common::DENSE_LINE_HEIGHT),
            common::DENSE_FONT,
            theme::LIGHT_GRAY,
        );

        let Ok(()) = TERMINAL
            .into_styled(PrimitiveStyle::with_fill(BLACK))
            .draw(canvas);
        for (row, text) in self.terminal.lines.iter().enumerate() {
            common::text(
                canvas,
                text,
                TERMINAL.top_left + Point::new(2, 2 + row as i32 * common::DENSE_LINE_HEIGHT),
                common::DENSE_FONT,
                TERMINAL_GREEN,
            );
        }

        button(canvas, ENROLL_BUTTON, "ENROLL", theme::LIGHT_BLUE);
        button(canvas, FORGET_BUTTON, "DEL", theme::DARK_GRAY);
        button(canvas, MINUS_BUTTON, "-", theme::DARK_GRAY);
        button(canvas, PLUS_BUTTON, "+", theme::DARK_GRAY);
    }
}

/// Upper-case a name for the banner.
trait UpperCase {
    /// The text in upper case, cut to the banner's width.
    fn to_ascii_uppercase_array(&self) -> ArrayString<16>;
}

impl UpperCase for str {
    fn to_ascii_uppercase_array(&self) -> ArrayString<16> {
        let mut text = ArrayString::new();
        for character in self.chars().take(16) {
            let _ = text.try_push(character.to_ascii_uppercase());
        }
        text
    }
}

/// Everything the panel shows. The panel is drawn again only when this
/// changes.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Status {
    /// What the app is doing.
    mode: Mode,
    /// People enrolled.
    people: u8,
    /// The settled decision.
    shown_name: Option<Option<u8>>,
    /// The last score, in hundredths.
    last_score: Option<i16>,
    /// The accept limit, in hundredths.
    accept: i16,
    /// What the user should do.
    hint: Hint,
    /// The last cycle's timing.
    timing: Timing,
    /// The terminal's revision.
    terminal: u32,
}

/// Draw a button: a filled box with a centred label.
fn button(canvas: &mut Canvas, area: Rectangle, label: &str, color: Rgb565) {
    let Ok(()) = area
        .into_styled(PrimitiveStyle::with_fill(color))
        .draw(canvas);
    common::centered_text(canvas, area, label, common::TITLE_FONT, theme::WHITE);
}

/// A value in hundredths, written as `0.35`.
struct Fixed(i16);

impl core::fmt::Display for Fixed {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let sign = if self.0 < 0 { "-" } else { "" };
        let value = self.0.unsigned_abs();
        write!(f, "{sign}{}.{:02}", value / 100, value % 100)
    }
}

/// The newest events, as lines of a small terminal on the panel.
struct Terminal {
    /// The lines, oldest first, each with a `> ` prompt.
    lines: ArrayVec<ArrayString<TERMINAL_COLUMNS>, TERMINAL_ROWS>,
    /// Changes with every new line, so the panel knows when to redraw.
    revision: u32,
}

impl Terminal {
    /// An empty terminal.
    fn new() -> Self {
        Self {
            lines: ArrayVec::new(),
            revision: 0,
        }
    }

    /// Log `text` and add it as the newest line.
    fn say(&mut self, text: &str) {
        self.say_fmt(format_args!("{text}"));
    }

    /// Like [`say`](Self::say), for formatted text. Text that does not fit
    /// on the line is cut off on the screen, not in the log.
    fn say_fmt(&mut self, text: core::fmt::Arguments<'_>) {
        info!("{text}");
        let mut line = ArrayString::new();
        let _ = write!(Cut(&mut line), "> {text}");
        if self.lines.is_full() {
            self.lines.remove(0);
        }
        self.lines.push(line);
        self.revision = self.revision.wrapping_add(1);
    }
}

/// Writes into a line until it is full, and drops the rest.
struct Cut<'a>(&'a mut ArrayString<TERMINAL_COLUMNS>);

impl core::fmt::Write for Cut<'_> {
    fn write_str(&mut self, text: &str) -> core::fmt::Result {
        for character in text.chars() {
            self.0.try_push(character).map_err(|_| core::fmt::Error)?;
        }
        Ok(())
    }
}

/// The middle columns of the newest camera frame, with the face box and the
/// landmarks drawn on top.
struct Preview<'a> {
    /// The newest camera frame.
    frame: Frame<'a>,
    /// The face to draw, in frame pixels, and the colour of its box.
    face: Option<(Face, Rgb565)>,
}

impl ScanlineSource for Preview<'_> {
    fn fill_row(&mut self, y: usize, row: &mut [u8]) {
        row.copy_from_slice(&self.frame.scanline(y)[PREVIEW_BYTES]);
        let Some((face, color)) = &self.face else {
            return;
        };
        let color_bytes = RawU16::from(*color).into_inner().to_be_bytes();
        let yellow = RawU16::from(YELLOW).into_inner().to_be_bytes();
        let y = y as i32;
        let (left, top) = (face.x as i32 - CROP_LEFT as i32, face.y as i32);
        let (right, bottom) = (left + face.width as i32, top + face.height as i32);
        if y == top || y == bottom {
            paint(row, left..right + 1, color_bytes);
        } else if y > top && y < bottom {
            paint(row, left..left + 1, color_bytes);
            paint(row, right..right + 1, color_bytes);
        }
        for [x, ly] in face.landmarks {
            let (x, ly) = (x as i32 - CROP_LEFT as i32, ly as i32);
            if (y - ly).abs() <= DOT {
                paint(row, x - DOT..x + DOT + 1, yellow);
            }
        }
    }

    fn while_transferring(&mut self) {
        self.frame.pump();
    }
}

/// Set the pixels in `columns` of a row of big-endian RGB565 bytes to
/// `color`; columns outside the row are skipped.
fn paint(row: &mut [u8], columns: Range<i32>, color: [u8; 2]) {
    let width = (row.len() / BYTES_PER_PIXEL) as i32;
    let start = columns.start.clamp(0, width) as usize;
    let end = columns.end.clamp(0, width) as usize;
    for pixel in
        row[start * BYTES_PER_PIXEL..end * BYTES_PER_PIXEL].chunks_exact_mut(BYTES_PER_PIXEL)
    {
        pixel.copy_from_slice(&color);
    }
}

/// The enrollments in the flash chip.
///
/// The last 128 KB of the 4 MB flash lie above the application image
/// (2.4 MB from offset 64 KB), so a new firmware does not touch them. The
/// layout, in 4 KB sectors:
///
/// - sector 0: a header: the magic `FACE`, the format version, the
///   number of people, and for each of the four slots the name (its
///   length, then 16 bytes) and the number of templates;
/// - sectors 1..: one block of 24 KB per slot: twelve embeddings of 512
///   `f32` values, little-endian.
///
/// Writing parks CPU1 for the duration (the flash cache is shared and
/// switched off while a sector is erased or written), so the audio and
/// radio tasks pause for about a second when an enrollment is saved.
mod store {
    use esp_hal::peripherals::FLASH;
    use esp_storage::{FlashStorage, FlashStorageError};
    use hack_and_hike_vision::gallery::{
        EMBEDDING_LEN, Embedding, Gallery, MAX_NAME, MAX_PEOPLE, MAX_TEMPLATES,
    };
    use log::warn;

    /// Where the store starts: the last 128 KB of a 4 MB flash.
    const BASE: u32 = 0x3E_0000;
    /// The sector size.
    const SECTOR: usize = 4096;
    /// One slot's block: twelve embeddings.
    const BLOCK: usize = MAX_TEMPLATES * EMBEDDING_LEN * 4;
    /// The header's magic.
    const MAGIC: [u8; 4] = *b"FACE";
    /// The format version.
    const VERSION: u32 = 1;
    /// Per slot in the header: the name's length, its bytes, the template
    /// count.
    const SLOT_HEADER: usize = 4 + MAX_NAME + 4;

    /// A sector-sized buffer on a word boundary, in internal RAM: the ROM
    /// routines read and write it while the cache is off.
    #[repr(C, align(4))]
    struct Sector([u8; SECTOR]);

    /// The flash, with a sector buffer.
    pub struct Store {
        /// The flash driver.
        flash: FlashStorage<'static>,
        /// The working sector.
        sector: alloc::boxed::Box<Sector>,
    }

    impl Store {
        /// Open the flash.
        pub fn open() -> Self {
            // SAFETY: nothing else in this firmware drives the flash chip
            // directly; the framework only executes from it through the
            // cache, which the driver switches off around each operation
            // after parking CPU1.
            let flash = FlashStorage::new(unsafe { FLASH::steal() }).multicore_auto_park();
            Self {
                flash,
                sector: alloc::boxed::Box::new(Sector([0; SECTOR])),
            }
        }

        /// Load the stored people into `gallery` (which must be empty).
        /// Returns how many.
        pub fn load(&mut self, gallery: &mut Gallery) -> Result<usize, FlashStorageError> {
            self.flash.read(BASE, &mut self.sector.0)?;
            let header = &self.sector.0;
            if header[..4] != MAGIC
                || u32::from_le_bytes([header[4], header[5], header[6], header[7]]) != VERSION
            {
                return Ok(0);
            }
            let count = u32::from_le_bytes([header[8], header[9], header[10], header[11]]) as usize;
            if count > MAX_PEOPLE {
                return Ok(0);
            }
            // The slots' names and template counts, copied out before the
            // sector buffer is reused for the blocks.
            let mut slots: [([u8; MAX_NAME], usize, usize); MAX_PEOPLE] =
                [([0; MAX_NAME], 0, 0); MAX_PEOPLE];
            for (slot, entry) in slots.iter_mut().enumerate().take(count) {
                let at = 12 + slot * SLOT_HEADER;
                let name_len = u32::from_le_bytes([
                    header[at],
                    header[at + 1],
                    header[at + 2],
                    header[at + 3],
                ]) as usize;
                let templates = u32::from_le_bytes([
                    header[at + 4 + MAX_NAME],
                    header[at + 5 + MAX_NAME],
                    header[at + 6 + MAX_NAME],
                    header[at + 7 + MAX_NAME],
                ]) as usize;
                if name_len == 0 || name_len > MAX_NAME || templates > MAX_TEMPLATES {
                    return Ok(0);
                }
                entry.0.copy_from_slice(&header[at + 4..at + 4 + MAX_NAME]);
                entry.1 = name_len;
                entry.2 = templates;
            }
            let mut loaded = 0;
            for (slot, (name, name_len, templates)) in slots.iter().enumerate().take(count) {
                let Ok(name) = core::str::from_utf8(&name[..*name_len]) else {
                    continue;
                };
                let Some(person) = gallery.enroll(name) else {
                    continue;
                };
                let mut values = [0.0f32; EMBEDDING_LEN];
                for template in 0..*templates {
                    let offset =
                        BASE + SECTOR as u32 + (slot * BLOCK + template * EMBEDDING_LEN * 4) as u32;
                    // An embedding is 2 KB: half a sector.
                    self.flash
                        .read(offset, &mut self.sector.0[..EMBEDDING_LEN * 4])?;
                    for (value, bytes) in values
                        .iter_mut()
                        .zip(self.sector.0[..EMBEDDING_LEN * 4].chunks_exact(4))
                    {
                        *value = f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
                    }
                    if values.iter().all(|v| v.is_finite()) && values.iter().any(|&v| v != 0.0) {
                        person.add_template(Embedding::from_raw(&values));
                    } else {
                        warn!("flash: template {template} of {name} is unreadable");
                    }
                }
                loaded += 1;
            }
            Ok(loaded)
        }

        /// Write the whole gallery: the header, then every enrolled
        /// person's block.
        pub fn save(&mut self, gallery: &Gallery) -> Result<(), FlashStorageError> {
            let people = gallery.people();
            for (slot, person) in people.iter().enumerate() {
                let templates = person.templates();
                // Two embeddings per sector.
                for (pair, chunk) in templates.chunks(2).enumerate() {
                    self.sector.0.fill(0xFF);
                    for (i, embedding) in chunk.iter().enumerate() {
                        for (bytes, value) in self.sector.0[i * EMBEDDING_LEN * 4..]
                            .chunks_exact_mut(4)
                            .zip(embedding.values())
                        {
                            bytes.copy_from_slice(&value.to_le_bytes());
                        }
                    }
                    let offset = BASE + SECTOR as u32 + (slot * BLOCK + pair * SECTOR) as u32;
                    self.write_sector(offset)?;
                }
            }
            self.sector.0.fill(0xFF);
            let header = &mut self.sector.0;
            header[..4].copy_from_slice(&MAGIC);
            header[4..8].copy_from_slice(&VERSION.to_le_bytes());
            header[8..12].copy_from_slice(&(people.len() as u32).to_le_bytes());
            for (slot, person) in people.iter().enumerate() {
                let at = 12 + slot * SLOT_HEADER;
                let name = person.name().as_bytes();
                header[at..at + 4].copy_from_slice(&(name.len() as u32).to_le_bytes());
                header[at + 4..at + 4 + MAX_NAME].fill(0);
                header[at + 4..at + 4 + name.len()].copy_from_slice(name);
                header[at + 4 + MAX_NAME..at + 8 + MAX_NAME]
                    .copy_from_slice(&(person.templates().len() as u32).to_le_bytes());
            }
            self.write_sector(BASE)
        }

        /// Write the working sector at `offset`.
        ///
        /// The driver parks CPU1 and then takes the critical section. If
        /// CPU1 held that lock when it was parked (its tasks take it all
        /// the time), this core would wait for it forever: the first
        /// build with the store hung that way. So the lock is taken here
        /// first: CPU1 can then only be parked while it waits for the
        /// lock, never while it holds it. The critical section is
        /// reentrant, so the driver's own nests inside.
        fn write_sector(&mut self, offset: u32) -> Result<(), FlashStorageError> {
            critical_section::with(|_| self.flash.write(offset, &self.sector.0))
        }
    }
}
