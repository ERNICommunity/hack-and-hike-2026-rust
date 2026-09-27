//! Face unlock. Record (enrol) your face once; after that, the board unlocks
//! when it sees your face again.
//!
//! The left part of the screen shows the camera, with an oval in the
//! middle. Hold your face so that it fills the oval: only the inside of the
//! oval counts, not the background around it. The right part shows the state
//! of the lock, a small terminal with what happened, and three buttons:
//!
//! - **ENROLL** records the face in the oval: five samples in two seconds.
//!   Move your head a little while it records. Enrol in the light in which
//!   you will unlock. Tap it again to replace the face.
//! - **-** and **+** change the limit: the largest distance between two
//!   faces that still counts as the same face. The status line shows the
//!   distance of each frame next to the limit, so you can find a good limit
//!   for your room: look at the distance for your face, for another face and
//!   for an empty oval. After enrolment, the terminal shows the spread: the
//!   largest distance between your own samples. A limit a bit above it is a
//!   good start.
//!
//! Three matching frames in a row unlock the board. It stays unlocked while
//! the face matches, and locks again five seconds after the last match. The
//! terminal lines also go to the log, so `espflash monitor` shows them too.
//!
//! The enrolled face and the limit are saved in flash, so they survive a
//! restart and a power-off. A new flash of `firmware.bin` erases them. Saving
//! takes about half a second and pauses the screen and the camera: it happens
//! once after enrolment, and two seconds after the last tap on **-** or **+**.
//!
//! How faces are compared is explained in `hack_and_hike_core::face`. It
//! is simple and has limits: there is no face detection, so the face must
//! be in the oval, and a photo of the face also unlocks.
//!
//! The camera's buffer overflows within a few milliseconds. So everything
//! happens while one camera frame is held, and every slow step calls
//! `frame.pump()` in between (see `hack_and_hike::capabilities::camera`).

#![no_std]
#![no_main]

use core::{fmt::Write as _, ops::Range};

use arrayvec::{ArrayString, ArrayVec};
use embassy_executor::Spawner;
use embassy_time::{Duration, Instant, Timer};
use embedded_graphics::{
    pixelcolor::{Rgb565, raw::RawU16},
    prelude::*,
    primitives::{PrimitiveStyle, Rectangle},
};
use hack_and_hike::{
    Board,
    capabilities::{
        camera::{self, Frame},
        display::{BYTES_PER_PIXEL, SCREEN, SIZE, ScanlineSource},
        storage::{Storage, StorageError},
        touch::TouchEvent,
    },
    psram,
    ui::{Canvas, common, theme},
};
use hack_and_hike_core::face::{
    self, CENTER_OFFSET, Codes, Event, FaceLock, Features, MASK_HALF_HEIGHT, MASK_HALF_WIDTH,
    Observation, PATCH_SIZE, Patch, Quality, SCALE, SOURCE_SIZE, State, TEMPLATES, Workspace,
};

// This line writes the application descriptor. The bootloader checks it
// before it starts the firmware. Every application needs this line exactly
// once.
esp_bootloader_esp_idf::esp_app_desc!();

/// The limit at start-up. In tests with photos of 158 people, fewer than 1
/// in 1,000 pairs of different people came closer than about 0.78. On the
/// board, other faces share your camera and light, so they may come closer:
/// change the limit with the buttons until your face unlocks and other
/// faces do not.
const DEFAULT_THRESHOLD: f32 = 0.75;
/// How long after the last tap on **-** or **+** the limit is saved, in
/// milliseconds. Tapping several times saves only once.
const SAVE_DELAY_MS: u64 = 2_000;
/// How much one tap on **-** or **+** changes the limit. The distances of
/// faces lie close together, so the steps are small.
const THRESHOLD_STEP: f32 = 0.01;

// The camera image and the face square.
/// Left edge of the face square in the camera image: the square is in the
/// middle.
const SOURCE_LEFT: usize = (camera::WIDTH - SOURCE_SIZE) / 2;
/// Top edge of the face square in the camera image.
const SOURCE_TOP: usize = (camera::HEIGHT - SOURCE_SIZE) / 2;
/// The bytes of a camera row that belong to the face square.
const SOURCE_BYTES: Range<usize> =
    SOURCE_LEFT * BYTES_PER_PIXEL..(SOURCE_LEFT + SOURCE_SIZE) * BYTES_PER_PIXEL;

// The screen: the camera on the left, the panel on the right.
/// Width of the camera preview, in pixels.
const PREVIEW_WIDTH: usize = 184;
/// The camera preview: the middle columns of the camera image.
const PREVIEW: Rectangle = Rectangle::new(
    Point::zero(),
    Size::new(PREVIEW_WIDTH as u32, camera::HEIGHT as u32),
);
/// The panel with the state, the terminal and the buttons.
const PANEL: Rectangle = Rectangle::new(
    Point::new(PREVIEW_WIDTH as i32, 0),
    Size::new(SIZE.width - PREVIEW_WIDTH as u32, SIZE.height),
);
/// Camera columns cut off on the left of the preview. As many are cut off on
/// the right.
const CROP_LEFT: usize = (camera::WIDTH - PREVIEW_WIDTH) / 2;
/// The bytes of each camera row that the preview shows.
const PREVIEW_BYTES: Range<usize> =
    CROP_LEFT * BYTES_PER_PIXEL..(CROP_LEFT + PREVIEW_WIDTH) * BYTES_PER_PIXEL;
// The oval shows the mask of the middle window, in which codes count. Code
// (x, y) belongs to patch pixel (x + 1, y + 1), the middle window starts at
// code `CENTER_OFFSET`, and the mask is centred in the window.
/// Centre of the oval in the preview, in pixels from the left.
const OVAL_CENTER_X: usize =
    SOURCE_LEFT + (CENTER_OFFSET + 1 + face::WINDOW / 2) * SCALE - CROP_LEFT;
/// Centre of the oval in the preview, in pixels from the top.
const OVAL_CENTER_Y: usize = SOURCE_TOP + (CENTER_OFFSET + 1 + face::WINDOW / 2) * SCALE;
/// Half the width of the oval, in camera pixels.
const OVAL_HALF_WIDTH: usize = MASK_HALF_WIDTH * SCALE;
/// Half the height of the oval, in camera pixels.
const OVAL_HALF_HEIGHT: usize = MASK_HALF_HEIGHT * SCALE;
/// Thickness of the oval's line, in pixels.
const OVAL_THICKNESS: usize = 3;

const _: () = assert!(camera::WIDTH == SCREEN.size.width as usize);
const _: () = assert!(camera::HEIGHT == SCREEN.size.height as usize);
const _: () = assert!(OVAL_CENTER_X >= OVAL_HALF_WIDTH);
const _: () = assert!(OVAL_CENTER_X + OVAL_HALF_WIDTH <= PREVIEW_WIDTH);
const _: () = assert!(OVAL_CENTER_Y >= OVAL_HALF_HEIGHT);
const _: () = assert!(OVAL_CENTER_Y + OVAL_HALF_HEIGHT <= camera::HEIGHT);

// The panel, in panel coordinates.
/// Left and right margin of the panel's text.
const MARGIN: i32 = 4;
/// The coloured box with the state of the lock.
const BANNER: Rectangle = Rectangle::new(Point::new(MARGIN, 24), Size::new(128, 30));
/// Top of the two status lines: distance and limit, then the image quality.
const STATUS_TOP: i32 = 60;
/// The terminal: the newest events, one per line.
const TERMINAL: Rectangle = Rectangle::new(Point::new(MARGIN, 88), Size::new(128, 100));
/// Lines in the terminal.
const TERMINAL_ROWS: usize = 8;
/// Characters per terminal line, including the `> ` prompt.
const TERMINAL_COLUMNS: usize = 21;
/// The button that starts the enrolment.
const ENROLL_BUTTON: Rectangle = Rectangle::new(Point::new(MARGIN, 196), Size::new(64, 40));
/// The button that lowers the limit: stricter.
const MINUS_BUTTON: Rectangle = Rectangle::new(Point::new(72, 196), Size::new(28, 40));
/// The button that raises the limit: more tolerant.
const PLUS_BUTTON: Rectangle = Rectangle::new(Point::new(104, 196), Size::new(28, 40));

// Colours that the theme does not have.
/// The banner and oval when unlocked.
const GREEN: Rgb565 = theme::rgb(0x2E9E4F);
/// The banner when locked.
const RED: Rgb565 = theme::rgb(0xC0392B);
/// The terminal's background.
const BLACK: Rgb565 = theme::rgb(0x000000);
/// The terminal's text.
const TERMINAL_GREEN: Rgb565 = theme::rgb(0x33FF66);

#[esp_rtos::main]
async fn main(_spawner: Spawner) -> ! {
    let Board {
        mut display,
        mut touch,
        camera,
        mut storage,
        ..
    } = Board::init();

    let Some(mut camera) = camera else {
        log::warn!("No camera found; face unlock needs it");
        let mut canvas = Canvas::new(SIZE);
        canvas.clear(theme::WHITE);
        let whole_screen = canvas.bounding_box();
        common::centered_text(
            &mut canvas,
            whole_screen,
            "No camera found",
            common::TITLE_FONT,
            theme::CHARCOAL,
        );
        canvas.show(&mut display.surface(SCREEN));
        loop {
            Timer::after(Duration::from_secs(1)).await;
        }
    };

    // The large buffers live in PSRAM, not on the stack.
    let patch = psram::leaked_value(Patch::new);
    let codes = psram::leaked_value(Codes::new);
    let mut workspace = Workspace::new(psram::leaked_slice(face::WORKSPACE_LEN, 0.0));
    let probe = psram::leaked_value(|| Features::EMPTY);
    let templates = psram::leaked_slice(TEMPLATES, Features::EMPTY);
    // The enrolment as bytes, as it is saved.
    let record = psram::leaked_slice(face::ENROLMENT_BYTES, 0_u8);

    let mut canvas = Canvas::new(PANEL.size);
    let mut terminal = Terminal::new();
    terminal.say("face unlock ready");
    let mut lock = load_enrolment(storage.as_mut(), record, templates, &mut terminal);
    let mut shown = None;
    // When to save the enrolment and the limit, in milliseconds.
    let mut save_at: Option<u64> = None;

    loop {
        while let Some(event) = touch.next_event() {
            if let TouchEvent::Pressed(point) = event
                && on_press(point - PANEL.top_left, &mut lock, &mut terminal)
                && lock.wants_distance()
            {
                // The limit of an enrolled face changed.
                save_at = Some(Instant::now().as_millis() + SAVE_DELAY_MS);
            }
        }

        let Some(mut frame) = camera.begin_frame() else {
            // The camera logged why. The next `begin_frame` tries again.
            embassy_futures::yield_now().await;
            continue;
        };
        let now_ms = Instant::now().as_millis();

        display.surface(PREVIEW).render_from(&mut Preview {
            frame: &mut frame,
            guide: guide_color(lock.state()),
        });

        // Look at the face square.
        for row in 0..PATCH_SIZE {
            let y = SOURCE_TOP + row * SCALE;
            patch.fill_row(
                row,
                &frame.scanline(y)[SOURCE_BYTES],
                &frame.scanline(y + 1)[SOURCE_BYTES],
            );
            frame.pump();
        }
        let quality = patch.quality();
        let observation = if quality == Quality::Good {
            face::prepare(patch, &mut workspace, codes, || frame.pump());
            let distance = if lock.wants_distance() {
                face::best_distance(codes, templates, probe, || frame.pump())
            } else {
                // The middle window, for an enrolment sample.
                probe.compute(codes, CENTER_OFFSET, CENTER_OFFSET);
                None
            };
            Observation::Face { distance }
        } else {
            Observation::NoFace
        };

        match lock.update(now_ms, observation) {
            Some(Event::SampleKept { index, complete }) => {
                templates[index].clone_from(probe);
                terminal.say_fmt(format_args!("sample {}/{} kept", index + 1, TEMPLATES));
                if complete {
                    terminal.say("face enrolled");
                    let spread = spread(templates, || frame.pump());
                    terminal.say_fmt(format_args!("spread {spread:.2}"));
                    save_at = Some(now_ms);
                }
            }
            Some(Event::Unlocked { distance }) => {
                terminal.say_fmt(format_args!("UNLOCKED d={distance:.2}"));
            }
            Some(Event::Locked) => terminal.say("locked again"),
            None => {}
        }

        let status = Status {
            state: lock.state(),
            distance: match observation {
                Observation::Face {
                    distance: Some(distance),
                } => Some(hundredths(distance)),
                _ => None,
            },
            threshold: hundredths(lock.threshold()),
            quality,
            terminal: terminal.revision,
        };
        if shown != Some(status) {
            shown = Some(status);
            draw_panel(&mut canvas, &status, &terminal, &mut || frame.pump());
            canvas.show_while(&mut display.surface(PANEL), || frame.pump());
        }

        frame.finish();

        if save_at.is_some_and(|due| now_ms >= due) {
            save_at = None;
            // Not while a new enrolment runs: it saves when it is complete.
            if let Some(storage) = storage.as_mut()
                && lock.wants_distance()
            {
                // The flash write stops the camera's capture, so pause it.
                // The next `begin_frame` starts it again.
                camera.pause();
                face::encode_enrolment(templates, lock.threshold(), record);
                match storage.save(face::ENROLMENT_RECORD, record) {
                    Ok(()) => terminal.say("face saved"),
                    Err(error) => {
                        log::warn!("Could not save the face: {error:?}");
                        terminal.say("saving failed");
                    }
                }
            }
        }

        // Let the other tasks run. Do not sleep: the camera buffer would
        // overflow.
        embassy_futures::yield_now().await;
    }
}

/// Load the saved enrolment into `templates`, and return the lock: closed
/// with the saved face, or empty when there is none. `record` is working
/// memory for the saved bytes.
fn load_enrolment(
    storage: Option<&mut Storage>,
    record: &mut [u8],
    templates: &mut [Features],
    terminal: &mut Terminal,
) -> FaceLock {
    let Some(storage) = storage else {
        terminal.say("no storage: not saved");
        terminal.say("tap ENROLL to start");
        return FaceLock::new(DEFAULT_THRESHOLD);
    };
    let loaded = match storage.load(face::ENROLMENT_RECORD, record) {
        Ok(bytes) => face::decode_enrolment(bytes, templates),
        // Nothing saved, or saved by another application or version.
        Err(StorageError::Empty | StorageError::OtherRecord) => None,
        Err(error) => {
            log::warn!("Could not load the saved face: {error:?}");
            None
        }
    };
    match loaded {
        Some(threshold) => {
            terminal.say("saved face loaded");
            FaceLock::enrolled(threshold)
        }
        None => {
            terminal.say("tap ENROLL to start");
            FaceLock::new(DEFAULT_THRESHOLD)
        }
    }
}

/// Handle a tap at `point`, in panel coordinates. Return whether the limit
/// changed.
fn on_press(point: Point, lock: &mut FaceLock, terminal: &mut Terminal) -> bool {
    let step = if ENROLL_BUTTON.contains(point) {
        lock.start_enrolling(Instant::now().as_millis());
        terminal.say("fill the oval");
        return false;
    } else if MINUS_BUTTON.contains(point) {
        -THRESHOLD_STEP
    } else if PLUS_BUTTON.contains(point) {
        THRESHOLD_STEP
    } else {
        return false;
    };
    lock.set_threshold(lock.threshold() + step);
    log::info!("limit {:.2}", lock.threshold());
    true
}

/// The largest distance between two enrolment samples. A limit a bit above
/// it is a good start. `between` runs after each comparison.
fn spread(templates: &[Features], mut between: impl FnMut()) -> f32 {
    let mut largest = 0.0_f32;
    for (index, first) in templates.iter().enumerate() {
        for second in &templates[index + 1..] {
            largest = largest.max(first.distance(second));
            between();
        }
    }
    largest
}

/// A distance in hundredths, rounded, as the panel shows it.
fn hundredths(value: f32) -> u16 {
    libm::roundf(value * 100.0).clamp(0.0, f32::from(u16::MAX)) as u16
}

/// The colour of the oval in each state.
fn guide_color(state: State) -> Rgb565 {
    match state {
        State::Enrolling { .. } => theme::LIGHT_BLUE,
        State::Unlocked { .. } => GREEN,
        State::Empty | State::Locked { .. } => theme::WHITE,
    }
}

/// Everything the panel shows. The panel is drawn again only when this
/// changes.
#[derive(Clone, Copy, PartialEq)]
struct Status {
    /// Where the lock is.
    state: State,
    /// The distance of the last frame, in hundredths; `None` when it was
    /// not computed.
    distance: Option<u16>,
    /// The limit, in hundredths.
    threshold: u16,
    /// The quality of the last frame.
    quality: Quality,
    /// The terminal's revision: it changes with every new line.
    terminal: u32,
}

/// Draw the whole panel onto `canvas`. `between` runs between the parts,
/// because drawing text pixel by pixel into PSRAM takes a few milliseconds.
fn draw_panel(
    canvas: &mut Canvas,
    status: &Status,
    terminal: &Terminal,
    between: &mut impl FnMut(),
) {
    canvas.clear(theme::CHARCOAL);
    common::text(
        canvas,
        "FACE UNLOCK",
        Point::new(MARGIN, 6),
        common::TITLE_FONT,
        theme::WHITE,
    );

    let mut label = ArrayString::<24>::new();
    let color = match status.state {
        State::Empty => {
            label.push_str("NOT ENROLLED");
            theme::DARK_GRAY
        }
        State::Enrolling { samples, .. } => {
            write!(label, "ENROLL {samples}/{TEMPLATES}").expect("the label fits");
            theme::LIGHT_BLUE
        }
        State::Locked { .. } => {
            label.push_str("LOCKED");
            RED
        }
        State::Unlocked { .. } => {
            label.push_str("UNLOCKED");
            GREEN
        }
    };
    let Ok(()) = BANNER
        .into_styled(PrimitiveStyle::with_fill(color))
        .draw(canvas);
    common::centered_text(canvas, BANNER, &label, common::TITLE_FONT, theme::WHITE);
    between();

    let mut line = ArrayString::<TERMINAL_COLUMNS>::new();
    match status.distance {
        Some(distance) => write!(line, "dist {} ", Fixed(distance)),
        None => write!(line, "dist --   "),
    }
    .expect("the status line fits");
    write!(line, "lim {}", Fixed(status.threshold)).expect("the status line fits");
    common::text(
        canvas,
        &line,
        Point::new(MARGIN, STATUS_TOP),
        common::DENSE_FONT,
        theme::WHITE,
    );
    let hint = match status.quality {
        Quality::Good => "face ok",
        Quality::TooDark => "too dark",
        Quality::TooBright => "too bright",
        Quality::Flat => "no contrast",
    };
    common::text(
        canvas,
        hint,
        Point::new(MARGIN, STATUS_TOP + common::DENSE_LINE_HEIGHT),
        common::DENSE_FONT,
        theme::LIGHT_GRAY,
    );
    between();

    let Ok(()) = TERMINAL
        .into_styled(PrimitiveStyle::with_fill(BLACK))
        .draw(canvas);
    for (row, text) in terminal.lines.iter().enumerate() {
        common::text(
            canvas,
            text,
            TERMINAL.top_left + Point::new(2, 2 + row as i32 * common::DENSE_LINE_HEIGHT),
            common::DENSE_FONT,
            TERMINAL_GREEN,
        );
        between();
    }

    button(canvas, ENROLL_BUTTON, "ENROLL", theme::LIGHT_BLUE);
    button(canvas, MINUS_BUTTON, "-", theme::DARK_GRAY);
    button(canvas, PLUS_BUTTON, "+", theme::DARK_GRAY);
    between();
}

/// Draw a button: a filled box with a centred label.
fn button(canvas: &mut Canvas, area: Rectangle, label: &str, color: Rgb565) {
    let Ok(()) = area
        .into_styled(PrimitiveStyle::with_fill(color))
        .draw(canvas);
    common::centered_text(canvas, area, label, common::TITLE_FONT, theme::WHITE);
}

/// A value in hundredths, written as `0.25`.
struct Fixed(u16);

impl core::fmt::Display for Fixed {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}.{:02}", self.0 / 100, self.0 % 100)
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

    /// Log `text` and add it as the newest line. The oldest line scrolls
    /// out when the terminal is full.
    fn say(&mut self, text: &str) {
        self.say_fmt(format_args!("{text}"));
    }

    /// Like [`say`](Self::say), for formatted text. Text that does not fit
    /// on the line is cut off on the screen, not in the log.
    fn say_fmt(&mut self, text: core::fmt::Arguments<'_>) {
        log::info!("{text}");
        let mut line = ArrayString::new();
        // `Cut` stops at the end of the line, so an error only means that
        // the text was cut.
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

/// The middle columns of each camera row, with the oval drawn on top. While
/// the LCD receives rows, the camera continues to capture.
struct Preview<'a, 'f> {
    /// The frame being shown, borrowed mutably so that `pump` can run
    /// during the transfer.
    frame: &'a mut Frame<'f>,
    /// The colour of the oval.
    guide: Rgb565,
}

impl ScanlineSource for Preview<'_, '_> {
    fn fill_row(&mut self, y: usize, row: &mut [u8]) {
        row.copy_from_slice(&self.frame.scanline(y)[PREVIEW_BYTES]);

        // The line of the oval lies between an outer and an inner ellipse.
        let color = RawU16::from(self.guide).into_inner().to_be_bytes();
        let outer = half_width(y, OVAL_HALF_WIDTH, OVAL_HALF_HEIGHT);
        let inner = half_width(
            y,
            OVAL_HALF_WIDTH - OVAL_THICKNESS,
            OVAL_HALF_HEIGHT - OVAL_THICKNESS,
        );
        let center = OVAL_CENTER_X as f32;
        let column = |x: f32| libm::roundf(x) as usize;
        match (outer, inner) {
            // The two sides of the line.
            (Some(outer), Some(inner)) => {
                let width = column(outer - inner).max(1);
                let left = column(center - outer);
                let right = column(center + outer);
                paint(row, left..left + width, color);
                paint(row, right - width..right, color);
            }
            // The top and bottom of the oval: above and below the inner
            // ellipse, the whole row between the outer edges.
            (Some(outer), None) => {
                paint(row, column(center - outer)..column(center + outer), color)
            }
            _ => {}
        }
    }

    fn while_transferring(&mut self) {
        self.frame.pump();
    }
}

/// Half the width of an ellipse with half-axes `half_width` and
/// `half_height`, centred on the oval's centre, in preview row `y`. `None`
/// when the row is above or below the ellipse.
fn half_width(y: usize, half_width: usize, half_height: usize) -> Option<f32> {
    let dy = (y as f32 + 0.5 - OVAL_CENTER_Y as f32) / half_height as f32;
    (dy.abs() < 1.0).then(|| half_width as f32 * libm::sqrtf(1.0 - dy * dy))
}

/// Set the pixels in `columns` of a row of big-endian RGB565 bytes to
/// `color`.
fn paint(row: &mut [u8], columns: Range<usize>, color: [u8; 2]) {
    let bytes = columns.start * BYTES_PER_PIXEL..columns.end * BYTES_PER_PIXEL;
    for pixel in row[bytes].chunks_exact_mut(BYTES_PER_PIXEL) {
        pixel.copy_from_slice(&color);
    }
}
