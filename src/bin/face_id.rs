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
//! A face that scores far above the limit is named on its first
//! embedding. A face that scores just above it waits for the average of
//! three embeddings and for a vote over the last decisions, which keeps
//! the name from flickering (`gallery::Decider`). Everything shown on the
//! panel also goes to the log.
//!
//! # How a cycle runs
//!
//! The detector takes about 150 ms and the recognizer about 600, and the
//! camera's buffer overflows within a few milliseconds if nobody empties
//! it. So the camera and the screen belong to a task on an interrupt
//! executor (see [`stream`]): it
//! interrupts the networks every few milliseconds to empty the camera's
//! buffer, and shows the live image about ten times per second, with the
//! box of the newest detection drawn on top.
//!
//! A cycle of the main task asks that task for the newest frame and gets
//! the frame's buffer in exchange for its own, without a copy. It works
//! on that frame: the detector on a 4x scaled-down version, the recognizer
//! on the face cut out of a 2x version. The box on the preview is
//! therefore up to one cycle old, while the image is live.
//!
//! # What keeps the cycle short
//!
//! - Both networks are compiled when the app starts: every tensor found,
//!   every plan made (`Model::compile`). A cycle only computes.
//! - The camera copies a frame out of its buffer only when the preview or
//!   the main task will use it, about ten times per second.
//! - While a preview is on its way to the screen, the stream task sleeps
//!   and the networks go on.
//! - The panel draws only the lines that changed, and the timings at most
//!   once per second.
//! - The copy of the screen for the live feed is made only while a
//!   computer watches the feed.
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

use alloc::boxed::Box;

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
    logging, psram,
    ui::{Canvas, common, theme},
};
use hack_and_hike_vision::{
    align::{CROP_SIZE, align_face, recognizer_input_i8, source_region},
    blob::Blob,
    detect::{
        CONTENT_HEIGHT, CONTENT_WIDTH, DEFAULT_NMS_THRESHOLD, DEFAULT_SCORE_THRESHOLD, DOWNSCALE,
        Face, decode, detector_input_i8,
    },
    gallery::{Decider, Embedding, Gallery, ImpostorBank, MAX_PEOPLE, SureSteps, Thresholds},
    gates::{self, Framing, Limits},
    image::{
        GrayImageMut, Rgb565Frame, RgbImageMut, downscale_to_rgb, downscale_to_rgb_within,
        rgb_to_gray,
    },
    include_fkb,
    nn::{
        BlobWeights, check, edgeface,
        lanes::{self, GeluTable, GroupPlan, NormPlan},
        pack, yunet,
    },
    quality::laplacian_variance,
};
use log::{info, warn};
use static_cell::StaticCell;

esp_bootloader_esp_idf::esp_app_desc!();

/// Which build this is, in the log and on the screen at start-up.
const BUILD_ID: &str = "faceid-14";

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
/// How much one tap on **-** or **+** changes the accept limit, in
/// hundredths.
const THRESHOLD_STEP: i16 = 2;
/// How long without a usable face until the banner goes back to
/// scanning.
const IDLE_RESET: Duration = Duration::from_secs(1);
/// How often the panel shows new timings at most. They change with every
/// cycle, and every change costs a drawing.
const TIMING_PERIOD: Duration = Duration::from_secs(1);
/// The recognizer's scaled-down source: half the camera frame.
const SOURCE_SCALE: usize = 2;
/// Width of the recognizer's source image.
const SOURCE_WIDTH: usize = camera::WIDTH / SOURCE_SCALE;
/// Height of the recognizer's source image.
const SOURCE_HEIGHT: usize = camera::HEIGHT / SOURCE_SCALE;
/// How often the live preview is drawn at most. A drawing takes about
/// 18 ms (the SPI transfer). The stream task sleeps while a batch of rows
/// is on the bus, but filling the batches still costs CPU0 about 14 ms
/// per preview.
const PREVIEW_PERIOD: Duration = Duration::from_millis(100);
/// Every how many previews one goes to the live screen feed. The feed shows
/// two to three camera frames per second, whatever the preview draws.
const MIRRORED_PREVIEWS: u32 = 4;
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
/// The status lines: the score, the hint, the timings, the people.
const STATUS_LINES: i32 = 4;
/// The terminal: the newest events, one per line.
const TERMINAL: Rectangle = Rectangle::new(Point::new(MARGIN, 112), Size::new(128, 76));
/// Lines in the terminal.
const TERMINAL_ROWS: usize = 6;
/// Characters per terminal line, including the `> ` prompt: a message
/// has 19.
const TERMINAL_COLUMNS: usize = 21;
/// The button that starts an enrollment.
const ENROLL_BUTTON: Rectangle = Rectangle::new(Point::new(MARGIN, 196), Size::new(58, 40));
/// The button that empties the gallery.
const FORGET_BUTTON: Rectangle = Rectangle::new(Point::new(66, 196), Size::new(34, 40));
/// The button that lowers the limit: more tolerant.
const MINUS_BUTTON: Rectangle = Rectangle::new(Point::new(104, 196), Size::new(14, 40));
/// The button that raises the limit: stricter.
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

/// The large buffers, all in PSRAM but one.
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
    /// A strip of the recognizer's MLP hidden tensor, in internal RAM:
    /// the kernels write each value once and read it once, which costs
    /// about 36 cycles per value in PSRAM (`Scratch::with_hidden`).
    hidden: &'static mut [i16],
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
            hidden: aligned_internal(edgeface::int8::HIDDEN_STRIP_LEN),
            embedding: psram::leaked_slice(edgeface::EMBEDDING_LEN, 0.0),
        }
    }
}

/// A copy of a weights file in PSRAM with its linear and convolution
/// weights grouped by eight output channels, the layout the vector unit
/// reads (`nn::pack`); PSRAM also delivers about four times flash's
/// bandwidth. Built one tensor at a time with a pause after each, so the
/// tasks on CPU1, which run from the same flash, keep their share of it:
/// a continuous copy starves them.
async fn packed_copy(file: &'static [u8]) -> &'static BlobWeights<'static> {
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
    Box::leak(Box::new(
        BlobWeights::new(copy).expect("the packed weights are a valid file"),
    ))
}

/// A copy of the impostor bank in PSRAM, which the decision reads four
/// times as fast as flash; in pieces with a pause after each, like the
/// weights.
async fn bank_copy() -> &'static [u8] {
    const PIECE: usize = 16 * 1024;
    let copy = aligned_psram::<u8>(IMPOSTORS.len(), 0);
    for (target, source) in copy.chunks_mut(PIECE).zip(IMPOSTORS.chunks(PIECE)) {
        target.copy_from_slice(source);
        Timer::after(Duration::from_millis(2)).await;
    }
    copy
}

/// The recognizer with every tensor found and every plan made. The model
/// and its plans are in PSRAM: in the main task they would take internal
/// RAM from the stack.
fn compile_recognizer(
    weights: &'static BlobWeights<'static>,
) -> &'static edgeface::int8::Model<'static> {
    let start = Instant::now();
    let storage = edgeface::int8::ModelStorage {
        plans: psram::leaked_slice(edgeface::int8::MODEL_PLANS, GroupPlan::ZERO),
        norm_plans: psram::leaked_slice(
            edgeface::int8::MODEL_NORM_PLANS,
            NormPlan::new(&[1.0; 8], &[0.0; 8], &[1.0; 8]),
        ),
        weights: aligned_psram(edgeface::int8::MODEL_WEIGHTS_LEN, 0),
        constants: aligned_psram(edgeface::int8::MODEL_CONSTANTS_LEN, 0),
        wide: aligned_psram(edgeface::int8::MODEL_WIDE_LEN, 0),
    };
    let model = psram::leaked_value(|| edgeface::int8::Model::compile(weights, storage));
    info!(
        "compiled the recognizer in {} ms",
        start.elapsed().as_millis()
    );
    model
}

/// The detector with every tensor found, in PSRAM like the recognizer.
fn compile_detector(
    weights: &'static BlobWeights<'static>,
) -> &'static yunet::int8::Model<'static> {
    let start = Instant::now();
    let storage = yunet::int8::ModelStorage {
        weights: aligned_psram(yunet::int8::MODEL_WEIGHTS_LEN, 0),
        plans: psram::leaked_slice(yunet::int8::MODEL_PLANS, GroupPlan::ZERO),
    };
    let model = psram::leaked_value(|| yunet::int8::Model::compile(weights, storage));
    info!(
        "compiled the detector in {} ms",
        start.elapsed().as_millis()
    );
    model
}

/// A PSRAM slice of `len` values that starts on a 16-byte boundary, as
/// the vector unit's loads need.
fn aligned_psram<T: Clone + 'static>(len: usize, value: T) -> &'static mut [T] {
    let spare = 16 / core::mem::size_of::<T>().max(1);
    let raw = psram::leaked_slice::<T>(len + spare, value);
    let skip = raw.as_ptr().align_offset(16);
    &mut raw[skip..skip + len]
}

/// [`aligned_psram`] in internal RAM, for `i16` values.
fn aligned_internal(len: usize) -> &'static mut [i16] {
    let raw = alloc::vec![0i16; len + 8].leak();
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
    let recognizer = compile_recognizer(packed_copy(EDGEFACE).await);
    let detector = compile_detector(packed_copy(YUNET).await);
    let impostors = Blob::parse(bank_copy().await).expect("the impostor bank is a valid file");
    let impostors = impostors.get("impostors").expect("the impostors tensor");
    let bank = ImpostorBank::from_i8(impostors.i8_slice(), impostors.scale);
    // The GELU table (15 KB), built once in internal RAM: its reads are
    // random, and internal RAM needs no cache (in PSRAM, the 64 KB table
    // of earlier builds missed the cache on half of them).
    let gelu = GeluTable::build(alloc::vec![0i16; GeluTable::LEN].leak());
    logging::report_memory("face id ready");
    // Before the camera starts: the networks alone on CPU0.
    let checked = check_networks(detector, recognizer, &gelu, &mut buffers);

    let mut app = App {
        thresholds: Thresholds::DEFAULT,
        sure: SureSteps::DEFAULT,
        limits: Limits::DEFAULT,
        mode: Mode::Scanning,
        decider: Decider::new(),
        last_score: None,
        hint: Hint::NoFace,
        timing: Timing::default(),
        shown_timing: Timing::default(),
        timing_shown_at: Instant::now(),
        last_usable: Instant::now(),
        terminal: Terminal::new(),
    };
    app.terminal.say_fmt(format_args!("face id {BUILD_ID}"));
    app.terminal.say(if checked {
        "self-test ok"
    } else {
        "SELF-TEST FAILED"
    });
    app.terminal.say("tap ENROLL to start");

    let mut canvas: &'static mut Canvas = psram::leaked_value(|| Canvas::new(PANEL.size));
    let mut shown: Option<Status> = None;
    // The copy of the screen for the live feed costs 6 ms per preview:
    // only while a computer watches.
    logging::mirror_only_when_watched(true);
    let mut feed_refreshes = logging::mirror_refreshes();

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

        // 1. The newest frame, from the stream task, for the buffer of the
        // frame before.
        let cycle_started = Instant::now();
        let previews_before = PREVIEWS_SHOWN.load(Ordering::Relaxed);
        let stream_before = STREAM_BUSY_US.load(Ordering::Relaxed);
        let dropped_before = BAD_FRAMES.load(Ordering::Relaxed);
        let copied_before = FRAMES_COPIED.load(Ordering::Relaxed);
        let copy_before = COPY_US.load(Ordering::Relaxed);
        FRAME_WANTED.signal(core::mem::take(&mut buffers.frame));
        buffers.frame = FRAME_TAKEN.wait().await;
        app.timing = Timing {
            capture_ms: cycle_started.elapsed().as_millis() as u32,
            ..Timing::default()
        };

        // 2. Detect.
        let face = detect(detector, &mut buffers, &mut app.timing);
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
            let embedding = embed(
                recognizer,
                &gelu,
                &judged.face,
                &mut buffers,
                &app.limits,
                &mut app.timing,
            );
            match embedding {
                Some(embedding) => {
                    embedded = true;
                    let started = Instant::now();
                    app.on_embedding(embedding, gallery, &bank, &mut store);
                    app.timing.decide_ms = started.elapsed().as_millis() as u32;
                }
                None => app.hint = Hint::Blurred,
            }
        }
        if embedded {
            app.last_usable = Instant::now();
        } else if !app.mode.is_enrolling() {
            // No embedding this cycle: the next average must not mix
            // faces from long ago with new ones, and after a moment
            // without a face the banner goes back to scanning.
            app.decider.pause();
            if app.last_usable.elapsed() >= IDLE_RESET {
                // Also when nothing is shown: the vote's decisions so far
                // must not count for the next face.
                if app.decider.shown().is_some() {
                    app.last_score = None;
                    app.terminal.say("face gone");
                }
                app.decider.clear();
            }
        }
        let cycle_ms = cycle_started.elapsed().as_millis().max(1) as u32;
        let previews = PREVIEWS_SHOWN
            .load(Ordering::Relaxed)
            .wrapping_sub(previews_before);
        let stream_ms = STREAM_BUSY_US
            .load(Ordering::Relaxed)
            .wrapping_sub(stream_before)
            / 1000;
        let dropped = BAD_FRAMES
            .load(Ordering::Relaxed)
            .wrapping_sub(dropped_before);
        let longest_gap_ms = LONGEST_PUMP_GAP_US.swap(0, Ordering::Relaxed) / 1000;
        let copied = FRAMES_COPIED
            .load(Ordering::Relaxed)
            .wrapping_sub(copied_before);
        let copy_ms = COPY_US.load(Ordering::Relaxed).wrapping_sub(copy_before) / 1000;
        info!(
            "cycle: {cycle_ms} ms: capture {}, scale {}, detect {}, align {}, embed {}, decide {}; stream {stream_ms} ms (copies of {copied} frames {copy_ms} ms), preview {:.1} fps, dropped {dropped}, longest pump gap {longest_gap_ms} ms; {}, {}",
            app.timing.capture_ms,
            app.timing.scale_ms,
            app.timing.detect_ms,
            app.timing.align_ms,
            app.timing.embed_ms,
            app.timing.decide_ms,
            previews as f32 * 1000.0 / cycle_ms as f32,
            match &judged {
                Some(judged) => judged.describe(),
                None => ArrayString::from("no face").expect("fits"),
            },
            app.hint.text()
        );

        // 5. The panel, when something changed: only what changed. A new
        // viewer of the live feed has none of it, so it gets all of it.
        let refreshes = logging::mirror_refreshes();
        if refreshes != feed_refreshes {
            feed_refreshes = refreshes;
            canvas.invalidate();
            shown = None;
        }
        let status = app.status(gallery);
        if shown != Some(status) {
            app.draw_panel(canvas, &status, shown.as_ref(), gallery);
            shown = Some(status);
            PANEL_WANTED.signal(canvas);
            canvas = PANEL_SHOWN.wait().await;
        }
        embassy_futures::yield_now().await;
    }
}

/// The main task's frame buffer, with the frame it has worked on: the
/// stream task gives it to the camera in exchange for the newest frame.
static FRAME_WANTED: Signal<CriticalSectionRawMutex, &'static mut [u8]> = Signal::new();
/// The buffer with the newest camera frame, for the main task.
static FRAME_TAKEN: Signal<CriticalSectionRawMutex, &'static mut [u8]> = Signal::new();
/// The drawn panel, for the stream task to show.
static PANEL_WANTED: Signal<CriticalSectionRawMutex, &'static mut Canvas> = Signal::new();
/// The same canvas back, once it is on the screen.
static PANEL_SHOWN: Signal<CriticalSectionRawMutex, &'static mut Canvas> = Signal::new();
/// The newest detection, in frame pixels, and the colour of its box.
static OVERLAY: Mutex<CriticalSectionRawMutex, Cell<Option<(Face, Rgb565)>>> =
    Mutex::new(Cell::new(None));
/// Preview frames drawn so far, for the frame rate in the log.
static PREVIEWS_SHOWN: AtomicU32 = AtomicU32::new(0);
/// The time CPU0 spent in the stream task so far, in microseconds: the
/// share of CPU0 that the networks do not get.
static STREAM_BUSY_US: AtomicU32 = AtomicU32::new(0);
/// Camera frames dropped so far.
static BAD_FRAMES: AtomicU32 = AtomicU32::new(0);
/// The longest time between two times the camera's buffer was emptied
/// since the main task last looked, in microseconds. The buffer overflows
/// at about 5 ms.
static LONGEST_PUMP_GAP_US: AtomicU32 = AtomicU32::new(0);
/// Camera frames copied out of the camera's buffer into PSRAM so far.
static FRAMES_COPIED: AtomicU32 = AtomicU32::new(0);
/// The time those copies took, in microseconds: a part of
/// [`STREAM_BUSY_US`].
static COPY_US: AtomicU32 = AtomicU32::new(0);

/// The camera and the screen, on an interrupt executor of CPU0.
///
/// The main task runs the networks for hundreds of milliseconds without a
/// pause. This task interrupts it every [`PUMP_PERIOD`] to empty the
/// camera's buffer, so the camera never stops, and draws the newest frame
/// with the [`OVERLAY`] every [`PREVIEW_PERIOD`]. It also serves the main
/// task: it hands the newest frame over for [`FRAME_WANTED`]'s buffer and
/// shows [`PANEL_WANTED`]'s canvas.
///
/// It never waits while it has CPU0. An interrupt handler that waits holds
/// up the task it interrupted, and the timer of both cores too, which has
/// the same priority: `faceid-10` waited for the camera after every
/// overflow of its buffer, 50 to 100 ms each time, and the IMU on CPU1
/// lost its samples. So the camera is started again without waiting
/// (`Camera::service`), and the preview and the panel sleep while their
/// pixels are on the bus.
///
/// It also takes as little of CPU0 as it can. The camera copies a frame
/// only when this task has a use for it: when the frame before was drawn
/// or handed over. Only every [`MIRRORED_PREVIEWS`]th preview goes to the
/// live screen feed. The time it takes is in [`STREAM_BUSY_US`].
#[embassy_executor::task]
async fn stream(camera: Camera, display: Display) -> ! {
    let mut body = core::pin::pin!(stream_loop(camera, display));
    core::future::poll_fn(|context| {
        let started = Instant::now();
        let poll = body.as_mut().poll(context);
        STREAM_BUSY_US.fetch_add(started.elapsed().as_micros() as u32, Ordering::Relaxed);
        poll
    })
    .await
}

/// The work of [`stream`].
async fn stream_loop(mut camera: Camera, mut display: Display) -> ! {
    camera.capture_on_demand(true);
    let mut next_preview = Instant::now();
    // Whether the camera's frame is new: neither drawn nor handed over.
    let mut fresh = false;
    let mut previews = 0u32;
    let mut frame_target = None;
    loop {
        camera.service();
        if camera.advance() {
            fresh = true;
        }
        if !fresh {
            // The frame is used up: a newer one is wanted. The camera
            // copies one frame for any number of requests.
            camera.request_frame();
        }
        BAD_FRAMES.store(camera.bad_frames(), Ordering::Relaxed);
        let stats = camera.take_stats();
        LONGEST_PUMP_GAP_US.fetch_max(stats.longest_gap.as_micros() as u32, Ordering::Relaxed);
        FRAMES_COPIED.fetch_add(stats.frames_copied, Ordering::Relaxed);
        COPY_US.fetch_add(stats.copy_time.as_micros() as u32, Ordering::Relaxed);
        if fresh && Instant::now() >= next_preview {
            // The drawing takes as long as the sensor needs for half a
            // frame: the frame it begins in that time is wanted already.
            camera.request_frame();
            if let Some(frame) = camera.current() {
                fresh = false;
                next_preview = Instant::now() + PREVIEW_PERIOD;
                previews = previews.wrapping_add(1);
                let face = OVERLAY.lock(Cell::get);
                let surface = display.surface(PREVIEW);
                let mut surface = if previews.is_multiple_of(MIRRORED_PREVIEWS) {
                    surface
                } else {
                    surface.without_mirror()
                };
                surface
                    .render_from_async(&mut Preview { frame, face })
                    .await;
                PREVIEWS_SHOWN.fetch_add(1, Ordering::Relaxed);
            }
        }
        if frame_target.is_none() {
            frame_target = FRAME_WANTED.try_take();
        }
        if let Some(target) = frame_target.take() {
            match camera.current() {
                Some(frame) => {
                    FRAME_TAKEN.signal(frame.take(target));
                    fresh = false;
                }
                None => frame_target = Some(target),
            }
        }
        if let Some(canvas) = PANEL_WANTED.try_take() {
            canvas
                .show_async(&mut display.surface(PANEL), || camera.service())
                .await;
            PANEL_SHOWN.signal(canvas);
        }
        Timer::after(PUMP_PERIOD).await;
    }
}

/// Run both networks once on the made-up inputs of `nn::check` and
/// compare their outputs with what the computer computes: `true` when
/// both agree bit for bit. Call it before the camera starts, so the log's
/// times are those of each network alone on CPU0.
fn check_networks(
    detector: &yunet::int8::Model<'_>,
    recognizer: &edgeface::int8::Model<'_>,
    gelu: &GeluTable<'_>,
    buffers: &mut Buffers,
) -> bool {
    check::noise(check::DETECTOR_SEED, buffers.detector_input);
    let started = Instant::now();
    let heads = detector.forward(
        buffers.detector_input,
        yunet::int8::Scratch::new(buffers.detector_i16, buffers.detector_f32),
    );
    let detector_ms = started.elapsed().as_millis();
    let detector_print = check::fingerprint(
        heads
            .iter()
            .flat_map(|head| [head.cls, head.obj, head.bbox, head.kps])
            .flatten()
            .copied(),
    );

    check::noise(check::RECOGNIZER_SEED, buffers.recognizer_input);
    let started = Instant::now();
    recognizer.forward(
        gelu,
        buffers.recognizer_input,
        edgeface::int8::Scratch::new(buffers.recognizer_i16, buffers.recognizer_f32)
            .with_hidden(buffers.hidden),
        buffers.embedding,
    );
    let recognizer_ms = started.elapsed().as_millis();
    let recognizer_print = check::fingerprint(buffers.embedding.iter().copied());
    profile_recognizer(recognizer, gelu, buffers);

    let same = detector_print == check::DETECTOR && recognizer_print == check::RECOGNIZER;
    // Lane kernel calls that did not run on the vector unit: none is
    // expected. (The detector's heads and its upsampling run on the
    // kernels of `quant`, and are not counted here.)
    let fallbacks = lanes::fallbacks();
    if same {
        info!(
            "self-test: detector {detector_ms} ms, recognizer {recognizer_ms} ms alone, {fallbacks} scalar fallbacks; both compute what the computer computes"
        );
    } else {
        warn!(
            "self-test: detector {detector_ms} ms, recognizer {recognizer_ms} ms alone, {fallbacks} scalar fallbacks; NOT what the computer computes: detector {detector_print:#018x} (computer {:#018x}), recognizer {recognizer_print:#018x} (computer {:#018x})",
            check::DETECTOR,
            check::RECOGNIZER
        );
    }
    same
}

/// Where the recognizer's time goes, from a pass with a trace: the time
/// between two trace calls is booked to the part the second one ends.
/// Each part includes the copy of its output to `f32` for the trace, so
/// the parts add up to a little more than a pass without one.
#[derive(Default)]
struct Profile {
    /// The stem, in microseconds.
    stem: u64,
    /// Each stage.
    stages: [StageProfile; 4],
    /// The head: pooling, LayerNorm, linear layer.
    head: u64,
}

/// The parts of one stage of a [`Profile`], in microseconds.
#[derive(Default)]
struct StageProfile {
    /// The downsample; stage 0 has none.
    downsample: u64,
    /// Each ConvBlock.
    blocks: ArrayVec<u64, 5>,
    /// The SplitTransposeBlock up to its attention's projection: the
    /// split convolutions, the positional encoding, the attention.
    attention: u64,
    /// The SplitTransposeBlock's MLP.
    mlp: u64,
}

impl Profile {
    /// Book `micros` to the part that the trace `name` ends (the names of
    /// `edgeface::int8::Model::forward_traced`).
    fn book(&mut self, name: &str, micros: u64) {
        if let Some(rest) = name.strip_prefix("stages.") {
            let Some(part) = rest
                .bytes()
                .next()
                .and_then(|digit| digit.checked_sub(b'0'))
                .and_then(|stage| self.stages.get_mut(usize::from(stage)))
            else {
                return;
            };
            if rest.contains(".downsample") {
                part.downsample += micros;
            } else if rest.ends_with(".Add") {
                let _ = part.blocks.try_push(micros);
            } else {
                // One of the SplitTransposeBlock's adds (`Add_1`, ...):
                // the last is its MLP's.
                part.attention += part.mlp;
                part.mlp = micros;
            }
        } else if name.starts_with("stem") {
            self.stem += micros;
        } else {
            self.head += micros;
        }
    }
}

/// Milliseconds with one decimal, from microseconds.
struct Millis(u64);

impl core::fmt::Display for Millis {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}.{}", self.0 / 1000, self.0 % 1000 / 100)
    }
}

/// Run the recognizer once more with a trace and log where its time goes,
/// one line per stage.
fn profile_recognizer(
    recognizer: &edgeface::int8::Model<'_>,
    gelu: &GeluTable<'_>,
    buffers: &mut Buffers,
) {
    let mut profile = Profile::default();
    let started = Instant::now();
    let mut last = started;
    recognizer.forward_traced(
        gelu,
        buffers.recognizer_input,
        edgeface::int8::Scratch::new(buffers.recognizer_i16, buffers.recognizer_f32)
            .with_hidden(buffers.hidden),
        buffers.embedding,
        |name, _, _| {
            let now = Instant::now();
            profile.book(name, (now - last).as_micros());
            last = now;
        },
    );
    info!(
        "profile: recognizer {} ms traced; stem {} ms, head {} ms",
        Millis(started.elapsed().as_micros()),
        Millis(profile.stem),
        Millis(profile.head)
    );
    for (index, stage) in profile.stages.iter().enumerate() {
        let mut blocks = ArrayString::<64>::new();
        for (block, &micros) in stage.blocks.iter().enumerate() {
            let separator = if block == 0 { "" } else { " + " };
            let _ = write!(blocks, "{separator}{}", Millis(micros));
        }
        let total =
            stage.downsample + stage.blocks.iter().sum::<u64>() + stage.attention + stage.mlp;
        info!(
            "profile: stage {index} {} ms: downsample {}, conv blocks {blocks}, attention {}, mlp {}",
            Millis(total),
            Millis(stage.downsample),
            Millis(stage.attention),
            Millis(stage.mlp)
        );
    }
}

/// Run the detector on the frame: the best face, in frame pixels.
fn detect(
    model: &yunet::int8::Model<'_>,
    buffers: &mut Buffers,
    timing: &mut Timing,
) -> Option<Face> {
    let started = Instant::now();
    let source = Rgb565Frame::new(buffers.frame, camera::WIDTH, camera::HEIGHT);
    let mut small = RgbImageMut::new(buffers.small, CONTENT_WIDTH, CONTENT_HEIGHT);
    downscale_to_rgb(&source, DOWNSCALE, &mut small);
    detector_input_i8(&small.as_image(), buffers.detector_input);
    timing.scale_ms = started.elapsed().as_millis() as u32;

    let started = Instant::now();
    let heads = model.forward(
        buffers.detector_input,
        yunet::int8::Scratch::new(buffers.detector_i16, buffers.detector_f32),
    );
    let faces = decode(&heads, DEFAULT_SCORE_THRESHOLD, DEFAULT_NMS_THRESHOLD);
    timing.detect_ms = started.elapsed().as_millis() as u32;
    faces.best().map(|face| face.scaled(DOWNSCALE as f32))
}

/// Cut the face out of the frame, check its sharpness and run the
/// recognizer. `None` when the crop is blurred or the landmarks are
/// degenerate.
fn embed(
    model: &edgeface::int8::Model<'_>,
    gelu: &GeluTable<'_>,
    face: &Face,
    buffers: &mut Buffers,
    limits: &Limits,
    timing: &mut Timing,
) -> Option<Embedding> {
    let started = Instant::now();
    let cut = cut_out(face, buffers, limits);
    timing.align_ms = started.elapsed().as_millis() as u32;
    cut?;

    let started = Instant::now();
    model.forward(
        gelu,
        buffers.recognizer_input,
        edgeface::int8::Scratch::new(buffers.recognizer_i16, buffers.recognizer_f32)
            .with_hidden(buffers.hidden),
        buffers.embedding,
    );
    timing.embed_ms = started.elapsed().as_millis() as u32;
    Some(Embedding::from_raw(buffers.embedding))
}

/// The recognizer's input: the face cut out of the frame, aligned by its
/// landmarks. `None` when the crop is blurred or the landmarks are
/// degenerate.
///
/// Only the part of the 2x scaled-down frame that the alignment reads is
/// scaled down (`align::source_region`); the rest of the buffer keeps an
/// older frame and is not read. The log gets the time of each step.
fn cut_out(face: &Face, buffers: &mut Buffers, limits: &Limits) -> Option<()> {
    let started = Instant::now();
    let landmarks = face
        .landmarks
        .map(|[x, y]| [x / SOURCE_SCALE as f32, y / SOURCE_SCALE as f32]);
    let (columns, rows) = source_region(&landmarks, SOURCE_WIDTH, SOURCE_HEIGHT)?;
    let region = (columns.len(), rows.len());
    let source = Rgb565Frame::new(buffers.frame, camera::WIDTH, camera::HEIGHT);
    let mut half = RgbImageMut::new(buffers.source, SOURCE_WIDTH, SOURCE_HEIGHT);
    downscale_to_rgb_within(&source, SOURCE_SCALE, &mut half, columns, rows);
    let scaled = started.elapsed();
    let mut crop = RgbImageMut::new(buffers.crop, CROP_SIZE, CROP_SIZE);
    align_face(&landmarks, &half.as_image(), &mut crop)?;
    let warped = started.elapsed();
    let mut gray = GrayImageMut::new(buffers.crop_gray, CROP_SIZE, CROP_SIZE);
    rgb_to_gray(&crop.as_image(), &mut gray);
    let sharpness = laplacian_variance(&gray.as_image());
    let judged = started.elapsed();
    if sharpness < limits.min_sharpness {
        info!("crop too blurred: sharpness {sharpness:.0}");
        return None;
    }
    recognizer_input_i8(&crop.as_image(), buffers.recognizer_input);
    let done = started.elapsed();
    info!(
        "align: {}x{} of {SOURCE_WIDTH}x{SOURCE_HEIGHT} scaled in {} us, warp {} us, sharpness {} us, input {} us",
        region.0,
        region.1,
        scaled.as_micros(),
        (warped - scaled).as_micros(),
        (judged - warped).as_micros(),
        (done - judged).as_micros()
    );
    Some(())
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

/// How long the steps of a cycle took; 0 for a step that did not run.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct Timing {
    /// Waiting for the frame.
    capture_ms: u32,
    /// Scaling the frame down for the detector.
    scale_ms: u32,
    /// The detector, with the decoding of its outputs.
    detect_ms: u32,
    /// Cutting the face out: scaling down, alignment, sharpness, and the
    /// recognizer's input.
    align_ms: u32,
    /// The recognizer.
    embed_ms: u32,
    /// The decision.
    decide_ms: u32,
}

/// The application's state.
struct App {
    /// The decision's limits.
    thresholds: Thresholds,
    /// How far above the limit a score is sure.
    sure: SureSteps,
    /// The gates' limits.
    limits: Limits,
    /// What the app is doing.
    mode: Mode,
    /// From the embeddings of the face in front of the camera to the name
    /// on the banner.
    decider: Decider,
    /// The score of the last decision, in hundredths.
    last_score: Option<i16>,
    /// What the user should do.
    hint: Hint,
    /// The last cycle's timing.
    timing: Timing,
    /// The timing on the panel: the detector's and the recognizer's of
    /// the last cycles that ran them.
    shown_timing: Timing,
    /// When the panel's timing was brought up to date.
    timing_shown_at: Instant,
    /// When the last embedding was made.
    last_usable: Instant,
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
            self.decider.clear();
            self.last_score = None;
            match store.save(gallery) {
                Ok(()) => self.terminal.say("gallery emptied"),
                Err(error) => {
                    warn!("flash: could not save: {error:?}");
                    self.terminal.say("flash save failed");
                }
            }
        } else if MINUS_BUTTON.contains(point) {
            self.step_limit(-THRESHOLD_STEP);
        } else if PLUS_BUTTON.contains(point) {
            self.step_limit(THRESHOLD_STEP);
        }
    }

    /// Move the accept limit by `step` hundredths, within 0 and 1. The
    /// limit stays a whole number of hundredths, as the panel shows it, so
    /// many taps add up to no error.
    fn step_limit(&mut self, step: i16) {
        let limit = (hundredths(self.thresholds.accept) + step).clamp(0, 100);
        self.thresholds.accept = f32::from(limit) / 100.0;
        self.terminal
            .say_fmt(format_args!("limit {:.2}", self.thresholds.accept));
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
            self.terminal.say("gallery full: DEL");
            return;
        };
        self.mode = Mode::Enrolling { index, samples: 0 };
        self.decider.clear();
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
                self.decider.clear();
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

        let decision = self
            .decider
            .push(embedding, gallery, bank, &self.thresholds, &self.sure);
        self.last_score = Some(hundredths(decision.score));
        let name = |index: Option<u8>| {
            index
                .and_then(|index| gallery.people().get(usize::from(index)))
                .map_or("unknown", |person| person.name())
        };
        info!(
            "decision: {}{} score {:.2} on {} frames (limit {:.2}, sure from {:.2})",
            decision.verdict.map_or("not yet", name),
            if decision.sure { ", sure," } else { "" },
            decision.score,
            decision.frames,
            self.thresholds.accept,
            self.sure.limit(self.thresholds.accept, decision.frames)
        );
        if let Some(shown) = decision.shown {
            match shown {
                Some(_) => self.terminal.say_fmt(format_args!("hello {}", name(shown))),
                None => self.terminal.say("unknown face"),
            }
        }
    }

    /// Everything the panel shows. The timings follow the cycles at most
    /// every [`TIMING_PERIOD`].
    fn status(&mut self, gallery: &Gallery) -> Status {
        if self.timing_shown_at.elapsed() >= TIMING_PERIOD {
            self.timing_shown_at = Instant::now();
            self.shown_timing.detect_ms = self.timing.scale_ms + self.timing.detect_ms;
            // The recognizer does not run in every cycle: its last time
            // stays.
            if self.timing.embed_ms != 0 {
                self.shown_timing.embed_ms = self.timing.align_ms + self.timing.embed_ms;
            }
        }
        Status {
            mode: self.mode,
            people: gallery.len() as u8,
            shown_name: self.decider.shown(),
            last_score: self.last_score,
            accept: hundredths(self.thresholds.accept),
            hint: self.hint,
            detect_ms: self.shown_timing.detect_ms,
            embed_ms: self.shown_timing.embed_ms,
            terminal: self.terminal.revision,
        }
    }

    /// Draw the panel onto `canvas`: all of it when `previous` is `None`,
    /// otherwise the parts whose content differs from `previous`. A part
    /// paints its whole area, so nothing of what it showed before stays.
    fn draw_panel(
        &self,
        canvas: &mut Canvas,
        status: &Status,
        previous: Option<&Status>,
        gallery: &Gallery,
    ) {
        if previous.is_none() {
            canvas.clear(theme::CHARCOAL);
            common::text(
                canvas,
                "FACE ID",
                Point::new(MARGIN, 6),
                common::TITLE_FONT,
                theme::WHITE,
            );
            button(canvas, ENROLL_BUTTON, "ENROLL", theme::LIGHT_BLUE);
            button(canvas, FORGET_BUTTON, "DEL", theme::DARK_GRAY);
            button(canvas, MINUS_BUTTON, "-", theme::DARK_GRAY);
            button(canvas, PLUS_BUTTON, "+", theme::DARK_GRAY);
        }

        if status.differs(previous, |s| (s.mode, s.shown_name, s.people)) {
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
        }

        let mut line = ArrayString::<TERMINAL_COLUMNS>::new();
        if status.differs(previous, |s| (s.last_score, s.accept)) {
            let _ = match status.last_score {
                Some(score) => write!(line, "score {} ", Fixed(score)),
                None => write!(line, "score --   "),
            };
            let _ = write!(line, "lim {}", Fixed(status.accept));
            status_line(canvas, 0, &line, theme::WHITE);
        }
        if status.differs(previous, |s| s.hint) {
            let color = if status.hint == Hint::Good {
                GREEN
            } else {
                theme::LIGHT_GRAY
            };
            status_line(canvas, 1, status.hint.text(), color);
        }
        if status.differs(previous, |s| (s.detect_ms, s.embed_ms)) {
            line.clear();
            let _ = write!(line, "det {}ms rec {}ms", status.detect_ms, status.embed_ms);
            status_line(canvas, 2, &line, theme::LIGHT_GRAY);
        }
        if status.differs(previous, |s| s.people) {
            line.clear();
            let _ = write!(line, "{} of {MAX_PEOPLE} enrolled", status.people);
            status_line(canvas, 3, &line, theme::LIGHT_GRAY);
        }

        if status.differs(previous, |s| s.terminal) {
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
        }
    }
}

/// Draw status line `row` (0 is the top one): its background over the
/// panel's whole width, then `text`.
fn status_line(canvas: &mut Canvas, row: i32, text: &str, color: Rgb565) {
    debug_assert!(row < STATUS_LINES);
    let top = STATUS_TOP + row * common::DENSE_LINE_HEIGHT;
    canvas.fill(
        Rectangle::new(
            Point::new(0, top),
            Size::new(PANEL.size.width, common::DENSE_LINE_HEIGHT as u32),
        ),
        theme::CHARCOAL,
    );
    common::text(
        canvas,
        text,
        Point::new(MARGIN, top),
        common::DENSE_FONT,
        color,
    );
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
/// changes, and only the parts that changed.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Status {
    /// What the app is doing.
    mode: Mode,
    /// People enrolled.
    people: u8,
    /// The person on the banner: their gallery index, or `None` for
    /// unknown; `None` as well before any decision.
    shown_name: Option<Option<u8>>,
    /// The last score, in hundredths.
    last_score: Option<i16>,
    /// The accept limit, in hundredths.
    accept: i16,
    /// What the user should do.
    hint: Hint,
    /// The detector's time, with the scaling of the frame.
    detect_ms: u32,
    /// The recognizer's time, with the cutting out of the face.
    embed_ms: u32,
    /// The terminal's revision.
    terminal: u32,
}

impl Status {
    /// Whether `part` of this status differs from the same part of
    /// `previous`; always when there is no `previous`.
    fn differs<T: PartialEq>(&self, previous: Option<&Self>, part: impl Fn(&Self) -> T) -> bool {
        previous.is_none_or(|previous| part(previous) != part(self))
    }
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

/// `value` in hundredths, rounded: how the panel shows the scores and
/// the limit.
fn hundredths(value: f32) -> i16 {
    libm::roundf(value * 100.0) as i16
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

    /// Log `text` and add it as the newest line. The screen shows 19
    /// characters of it.
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
/// (2.4 MB from offset 64 KB), so a new firmware does not touch them:
/// `cargo dist` writes an image that ends with the application
/// (`--skip-padding`), and autoflash writes only that. An image padded to
/// the size of the flash would erase them. The layout, in 4 KB sectors:
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

    /// A sector-sized buffer on a word boundary, for the header and the
    /// embeddings on their way to and from the flash. `esp-storage`
    /// copies it through a buffer of its own for the ROM routines.
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
