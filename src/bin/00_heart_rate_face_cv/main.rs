//! Heart rate from the face: camera, face detection and the green signal.
//!
//! See `PLAN.md` in this folder. Step 4: the camera on the left half (every
//! `RENDER_EVERY`th frame). Each frame gives one sample: the mean green
//! value of the whole view, which the user fills with their face. The right
//! half shows the samples per second, the green signal of the last 20 s,
//! and the heart rate with the spectrum of the last 10 s.
//!
//! Exposure and white balance are locked 3 s after start-up. A tap unlocks
//! them, and they are locked again 3 s later.

#![no_std]
#![no_main]

mod dashboard;
mod signal;
mod view;

use core::fmt::Write as _;

use arrayvec::ArrayString;
use embassy_executor::Spawner;
use embassy_futures::yield_now;
use embassy_time::{Duration, Instant, Timer};
use embedded_graphics::{prelude::*, primitives::Rectangle};
use hack_and_hike::{
    Board,
    capabilities::{camera, display, touch::TouchEvent},
};

use dashboard::Dashboard;
use signal::{History, Sample};
use view::CameraView;

esp_bootloader_esp_idf::esp_app_desc!();

/// The left half of the screen: the camera image.
const CAMERA_AREA: Rectangle = Rectangle::new(
    Point::zero(),
    Size::new(view::WIDTH as u32, display::HEIGHT as u32),
);
/// The right half of the screen: the dashboard.
const DASHBOARD_AREA: Rectangle =
    Rectangle::new(Point::new(view::WIDTH as i32, 0), dashboard::SIZE);
/// How often the FPS value is measured.
const FPS_PERIOD: Duration = Duration::from_secs(1);
/// How often the heart rate is computed and the dashboard is drawn again.
const DASHBOARD_PERIOD: Duration = Duration::from_millis(500);
/// Show every `RENDER_EVERY`th frame on the left half. The other frames are
/// only read for their green value, which is much faster than sending them.
const RENDER_EVERY: u32 = 3;
/// How long the camera adjusts exposure and white balance by itself, after
/// start-up or a tap, before they are locked.
const LOCK_DELAY: Duration = Duration::from_secs(3);
/// A `begin_frame` that takes longer than this had to start the capture
/// again, because a frame was dropped before it. Normally it returns at
/// once.
const RESTART_THRESHOLD: Duration = Duration::from_millis(10);

// The camera image fills the height of the screen, and both halves fill its
// width.
const _: () = assert!(camera::HEIGHT == display::HEIGHT);
const _: () = assert!(view::WIDTH + dashboard::SIZE.width as usize == display::WIDTH);

#[esp_rtos::main]
async fn main(_spawner: Spawner) -> ! {
    let Board {
        mut display,
        mut touch,
        camera,
        ..
    } = Board::init();

    let Some(mut camera) = camera else {
        log::error!("No camera detected. This app needs the camera.");
        loop {
            Timer::after_secs(1).await;
        }
    };

    let mut dashboard = Dashboard::new();
    let mut history = History::new();

    // Frames since `fps_since`.
    let mut frames: u32 = 0;
    let mut fps_since = Instant::now();
    let mut fps_tenths: u32 = 0;
    // `None` draws the dashboard at once, in the first loop iteration.
    let mut dashboard_since: Option<Instant> = None;
    // Counts all frames, to show every `RENDER_EVERY`th one.
    let mut frame_number: u32 = 0;
    // When exposure and white balance get locked. `None` once they are.
    let mut lock_at: Option<Instant> = Some(Instant::now() + LOCK_DELAY);
    // Capture restarts since start-up. Each one means a dropped frame.
    let mut drops: u32 = 0;

    loop {
        let start = Instant::now();
        if let Some(mut frame) = camera.begin_frame() {
            let time = Instant::now();
            if time.duration_since(start) > RESTART_THRESHOLD {
                drops += 1;
            }

            // Every `RENDER_EVERY`th frame goes to the left half, and the
            // view adds up the green values on the way. The other frames
            // are only read for their green value.
            let render = frame_number.is_multiple_of(RENDER_EVERY);
            frame_number = frame_number.wrapping_add(1);
            let green = if render {
                let mut camera_view = CameraView::new(&mut frame);
                display.surface(CAMERA_AREA).render_from(&mut camera_view);
                camera_view.green_mean()
            } else {
                Some(view::green_mean(&mut frame))
            };
            if let Some(green) = green {
                history.push(Sample { time, green });
            }
            frame.finish();
            frames += 1;
        }

        // Once per second: frames per elapsed time, in tenths.
        let elapsed = fps_since.elapsed();
        if elapsed >= FPS_PERIOD {
            fps_tenths = (u64::from(frames) * 10_000 / elapsed.as_millis()) as u32;
            frames = 0;
            fps_since = Instant::now();
        }

        // A tap switches auto adjust on again, and it is locked again
        // `LOCK_DELAY` later. Tap once your face fills the view.
        while let Some(event) = touch.next_event() {
            if let TouchEvent::Pressed(_) = event {
                camera.set_auto_adjust(true);
                lock_at = Some(Instant::now() + LOCK_DELAY);
            }
        }
        if lock_at.is_some_and(|at| Instant::now() >= at) {
            camera.set_auto_adjust(false);
            lock_at = None;
        }

        if dashboard_since.is_none_or(|since| since.elapsed() >= DASHBOARD_PERIOD) {
            let now = Instant::now();
            dashboard_since = Some(now);

            let mut status = ArrayString::<32>::new();
            let written = match lock_at {
                Some(at) => {
                    let left_ms = at
                        .checked_duration_since(now)
                        .map_or(0, |left| left.as_millis());
                    // Round up, so the count ends at 1, not 0.
                    let left_s = left_ms.div_ceil(1000);
                    write!(status, "exposure auto, lock in {left_s}s")
                }
                None => write!(status, "exposure locked, tap=redo"),
            };
            written.expect("the text fits its buffer");

            // The heart rate. The camera is pumped while it is computed.
            let spectrum = signal::spectrum(&history, now, || camera.pump());
            let dft_us = now.elapsed().as_micros();

            let mut footer = ArrayString::<32>::new();
            write!(
                footer,
                "dft {}.{} ms  drops {drops}",
                dft_us / 1000,
                dft_us % 1000 / 100
            )
            .expect("the text fits its buffer");

            let surface = &mut display.surface(DASHBOARD_AREA);
            dashboard.show(
                surface,
                &mut camera,
                fps_tenths,
                &history,
                now,
                &status,
                spectrum.as_ref(),
                &footer,
            );
        }

        // Empty the camera's ring buffer. The loop must not sleep, or the
        // ring overflows. `yield_now` lets other tasks on CPU0 run, without
        // waiting.
        camera.pump();
        yield_now().await;
    }
}
