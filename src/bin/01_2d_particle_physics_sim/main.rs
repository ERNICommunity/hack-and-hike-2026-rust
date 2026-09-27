//! Sand and water in a box: tilt or shake the board and the grains follow.
//!
//! See `PLAN.md` in this folder. The accelerometer gives the gravity that
//! the grains feel, `sim` moves them on a grid, `sound` plays a drag noise
//! that follows how much they move. A long press switches sand and water.

#![no_std]
#![no_main]

// `Vec` for the grains: the heap is in fast internal RAM.
extern crate alloc;

mod sim;
mod sound;

use core::fmt::Write as _;

use arrayvec::ArrayString;
use embassy_executor::Spawner;
use embassy_time::{Duration, Instant, Timer};
use embedded_graphics::{pixelcolor::Rgb565, prelude::*, primitives::Rectangle};
use hack_and_hike::{
    Board,
    capabilities::{display, touch::TouchEvent},
    ui::{
        Canvas,
        common::{DENSE_FONT, text},
        theme,
    },
};

use sim::{Material, World};
use sound::DragNoise;

esp_bootloader_esp_idf::esp_app_desc!();

/// Size of one cell (one grain) on the screen, in pixels.
const CELL_PX: usize = 4;
/// Height of the status line at the top.
const STATUS_HEIGHT: usize = 16;
/// The status line.
const STATUS_AREA: Rectangle = Rectangle::new(
    Point::zero(),
    Size::new(display::WIDTH as u32, STATUS_HEIGHT as u32),
);
/// The box with the grains, below the status line.
const BOX_AREA: Rectangle = Rectangle::new(
    Point::new(0, STATUS_HEIGHT as i32),
    Size::new(
        (sim::COLUMNS * CELL_PX) as u32,
        (sim::ROWS * CELL_PX) as u32,
    ),
);
// The status line and the box fill the screen exactly.
const _: () = assert!(sim::COLUMNS * CELL_PX == display::WIDTH);
const _: () = assert!(STATUS_HEIGHT + sim::ROWS * CELL_PX == display::HEIGHT);

/// How many grains. The box has 80 x 56 = 4,480 cells.
const GRAIN_COUNT: usize = 1_200;
/// Cells/s² per m/s² of acceleration: 9.8 m/s² → ~245 cells/s², so a grain
/// falls through the whole box in about 0.7 s.
const CELLS_PER_M_S2: f32 = 25.0;
/// Tilts below this (m/s² in the screen plane, ≈ 2°) count as flat, so
/// sensor noise does not make the grains creep.
const DEAD_ZONE_M_S2: f32 = 0.4;
/// One physics step. Fixed, so the simulation behaves the same at any FPS.
/// Longer steps are cheaper; the speed limit (under 1 cell per step) is
/// then 158 cells/s, about the fastest fall in this box.
const STEP: Duration = Duration::from_millis(6);
/// At most this much time is simulated per frame, so a slow frame does
/// not trigger a long catch-up.
const MAX_CATCH_UP: Duration = Duration::from_millis(48);
/// Motion (cell changes per second) below which it is silent...
const QUIET_MOVES_PER_S: f32 = 40.0;
/// ...and at which the noise is at full volume.
const LOUD_MOVES_PER_S: f32 = 20_000.0;
/// Hold a finger this long to switch sand and water. A plain tap does
/// nothing, because taps happen by accident while holding the board.
const LONG_PRESS: Duration = Duration::from_millis(800);
/// How often the status line is redrawn.
const STATUS_PERIOD: Duration = Duration::from_millis(500);

/// Colour of an empty cell.
const BACKGROUND: Rgb565 = theme::CHARCOAL;
/// Sand colours, from resting to fast.
const SAND: [Rgb565; 4] = [
    theme::rgb(0xC8A060),
    theme::rgb(0xDDB878),
    theme::rgb(0xF0D498),
    theme::rgb(0xFFF2D0),
];
/// Water colours, from resting to fast.
const WATER: [Rgb565; 4] = [
    theme::rgb(0x1E5AA8),
    theme::rgb(0x2E7AD0),
    theme::rgb(0x5AA0F0),
    theme::rgb(0xBFE4FF),
];

#[esp_rtos::main]
async fn main(_spawner: Spawner) -> ! {
    let Board {
        mut display,
        mut touch,
        mut imu,
        mut speaker,
        ..
    } = Board::init();

    let mut world = World::new(GRAIN_COUNT);
    let mut noise = DragNoise::default();
    let mut status = Canvas::new(STATUS_AREA.size);

    let mut gravity = [0.0_f32; 2];
    let mut press_since: Option<Instant> = None;
    let mut last = Instant::now();
    let mut lag = Duration::from_ticks(0);
    let mut volume = 0.0_f32;

    // Totals since the last status line.
    let mut status_since = Instant::now();
    let mut frames = 0_u32;
    let mut moves = 0_u32;
    let mut physics_us = 0_u64;
    let mut draw_us = 0_u64;

    loop {
        // 1. Input: the newest IMU sample, and touches for the long press.
        if let Some(sample) = imu.latest()
            && let Some([ax, ay, _]) = sample.acceleration_m_s2
        {
            gravity = screen_gravity(ax, ay);
        }
        while let Some(event) = touch.next_event() {
            match event {
                TouchEvent::Pressed(_) => press_since = Some(Instant::now()),
                TouchEvent::Released(_) => press_since = None,
                TouchEvent::Moved(_) => {}
            }
        }
        if press_since.is_some_and(|since| since.elapsed() >= LONG_PRESS) {
            world.material = world.material.toggled();
            press_since = None;
        }

        // 2. Physics: as many fixed steps as the time since the last frame.
        let physics_start = Instant::now();
        lag = (lag + physics_start.duration_since(last)).min(MAX_CATCH_UP);
        let loop_s = physics_start.duration_since(last).as_micros() as f32 / 1e6;
        last = physics_start;
        let step_s = STEP.as_micros() as f32 / 1e6;
        let mut moved = 0;
        while lag >= STEP {
            moved += world.step(gravity, step_s);
            lag -= STEP;
        }
        moves += moved;
        physics_us += physics_start.elapsed().as_micros();

        // 3. Sound: volume from the motion, then top up the speaker queue.
        if loop_s > 0.0 {
            let loudness = (moved as f32 / loop_s - QUIET_MOVES_PER_S)
                / (LOUD_MOVES_PER_S - QUIET_MOVES_PER_S);
            // sqrt makes small movements audible without loud big ones.
            volume = libm::sqrtf(loudness.clamp(0.0, 1.0));
        }
        noise.fill(&mut speaker, volume);

        // 4. Draw the box, row by row, straight from the grid.
        let draw_start = Instant::now();
        let (palette, gaps) = match world.material {
            Material::Sand => (&SAND, true),
            Material::Water => (&WATER, false),
        };
        // The colours of one row of cells, worked out once and reused for
        // its CELL_PX pixel rows.
        let mut colors = [BACKGROUND; sim::COLUMNS];
        display.surface(BOX_AREA).render_scanlines(|y, row| {
            if y.is_multiple_of(CELL_PX) {
                let cy = y / CELL_PX;
                for (cx, color) in colors.iter_mut().enumerate() {
                    *color = world
                        .speed_level(cx, cy)
                        .map_or(BACKGROUND, |level| palette[level]);
                }
            }
            // Sand: a 1-pixel gap around each grain makes it look grainy.
            if gaps && y % CELL_PX == CELL_PX - 1 {
                row.fill(BACKGROUND);
                return;
            }
            for (pixels, &color) in row.chunks_exact_mut(CELL_PX).zip(&colors) {
                pixels.fill(color);
                if gaps {
                    pixels[CELL_PX - 1] = BACKGROUND;
                }
            }
        });
        draw_us += draw_start.elapsed().as_micros();
        // Drawing took ~30 ms of the queue's 64 ms: top it up again.
        noise.fill(&mut speaker, volume);
        frames += 1;

        // 5. Status line, twice per second.
        let elapsed = status_since.elapsed();
        if elapsed >= STATUS_PERIOD {
            let seconds = elapsed.as_micros() as f32 / 1e6;
            let mut line = ArrayString::<64>::new();
            write!(
                line,
                "{:.0} fps  phys {}ms  draw {}ms  {} moves/s  {}",
                frames as f32 / seconds,
                physics_us / 1000 / u64::from(frames),
                draw_us / 1000 / u64::from(frames),
                (moves as f32 / seconds) as u32,
                world.material.name(),
            )
            .expect("the status fits its buffer");
            status.clear(theme::DARK_BLUE);
            text(
                &mut status,
                &line,
                Point::new(4, 2),
                DENSE_FONT,
                theme::WHITE,
            );
            status.show(&mut display.surface(STATUS_AREA));

            status_since = Instant::now();
            frames = 0;
            moves = 0;
            physics_us = 0;
            draw_us = 0;
        }

        // Every loop needs an `.await`. Drawing already takes ~30 ms, so a
        // short sleep is enough.
        Timer::after(Duration::from_millis(1)).await;
    }
}

/// Turn the accelerometer's in-screen components (m/s²) into the grains'
/// gravity in cells/s², `[right, down]`.
///
/// The library's screen frame has x out of the top edge and y to the right.
/// Screen pixels have x to the right and y down. So right = +y, down = −x.
fn screen_gravity(ax: f32, ay: f32) -> [f32; 2] {
    let (right, down) = (ay, -ax);
    if libm::hypotf(right, down) < DEAD_ZONE_M_S2 {
        return [0.0, 0.0];
    }
    [right * CELLS_PER_M_S2, down * CELLS_PER_M_S2]
}
