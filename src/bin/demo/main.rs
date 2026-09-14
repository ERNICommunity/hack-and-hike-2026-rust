//! Full Hack & Hike demo: one screen per capability plus settings and a log.
//!
//! `main` brings up the board, hands each screen the handles it owns, and
//! runs the loop: route touches, update every screen, draw the visible one.
//!
//! ```text
//! ┌────┬───────────────────────────┐
//! │ 📶 │                           │
//! │ 🧭 │   the visible screen      │
//! │ 🎤 │   (content area,          │
//! │ 🔊 │    276 x 240 pixels)      │
//! │ 📷 │                           │
//! │ ⚙  │                           │
//! │ 📄 │                           │
//! └────┴───────────────────────────┘
//!  rail: navigation.rs
//! ```
//!
//! The modules:
//!
//! - `layout`: where the rail and the content area are.
//! - `navigation`: the rail's icons and the routing of touches.
//! - `screens`: the [`Screen`] trait and one module per screen.
//! - `styles`: the widget styles the KDL layout files refer to.

#![no_std]
#![no_main]
#![warn(clippy::missing_docs_in_private_items)]

mod layout;
mod navigation;
mod screens;
mod styles;

use embassy_executor::Spawner;
use embassy_time::{Duration, Instant, Timer};
use hack_and_hike::{Board, ui::Canvas};

use navigation::{Navigation, ViewId};
use screens::{
    Screen, camera::CameraScreen, imu::ImuScreen, log::LogScreen, microphone::MicrophoneScreen,
    network::NetworkScreen, settings::SettingsScreen, speaker::SpeakerScreen,
};

// Writes the application descriptor the bootloader checks before starting
// the firmware. Every application needs this line exactly once.
esp_bootloader_esp_idf::esp_app_desc!();

/// Pause between loop iterations. Short enough for the camera and the IMU
/// screen to feel live, long enough to let CPU0 tasks run.
const LOOP_PERIOD: Duration = Duration::from_millis(2);

/// Every screen, one field each. A struct rather than an array, because the
/// screens are different types.
struct Screens {
    /// ESP-NOW pings and pongs, and the peers in range.
    network: NetworkScreen,
    /// Roll, pitch and yaw, with a 3-D horizon and compass.
    imu: ImuScreen,
    /// The live waveform of both microphone channels.
    microphone: MicrophoneScreen,
    /// The melody and chime player; it keeps playing on other screens.
    speaker: SpeakerScreen,
    /// The live camera preview.
    camera: CameraScreen,
    /// The display brightness slider.
    settings: SettingsScreen,
    /// The newest lines of the device log.
    log: LogScreen,
}

impl Screens {
    /// The screen for `id`, as a trait object: the shell calls the same
    /// methods on every screen without knowing its type.
    fn get_mut(&mut self, id: ViewId) -> &mut dyn Screen {
        match id {
            ViewId::Network => &mut self.network,
            ViewId::Imu => &mut self.imu,
            ViewId::Microphone => &mut self.microphone,
            ViewId::Speaker => &mut self.speaker,
            ViewId::Camera => &mut self.camera,
            ViewId::Settings => &mut self.settings,
            ViewId::Log => &mut self.log,
        }
    }

    /// Let every screen do its background work, visible or not.
    fn update_all(&mut self, now: Instant, slowest: &mut Slowest) {
        for id in ViewId::ALL {
            let started = Instant::now();
            self.get_mut(id).update(now);
            slowest.record(UPDATE_NAMES[id as usize], started.elapsed());
        }
    }
}

/// DIAGNOSTIC (temporary): names of the update phases, indexed by `ViewId`.
const UPDATE_NAMES: [&str; 7] = [
    "update Network",
    "update Imu",
    "update Microphone",
    "update Speaker",
    "update Camera",
    "update Settings",
    "update Log",
];

/// DIAGNOSTIC (temporary): the slowest loop phase seen since the last report.
struct Slowest {
    /// Name and duration of the slowest phase so far.
    phase: Option<(&'static str, Duration)>,
    /// When the last report was logged; reports are at most one per second
    /// so the logging itself does not slow the loop.
    last_report: Instant,
}

impl Slowest {
    /// Remember `elapsed` if it is the longest so far.
    fn record(&mut self, name: &'static str, elapsed: Duration) {
        if self.phase.is_none_or(|(_, longest)| elapsed > longest) {
            self.phase = Some((name, elapsed));
        }
    }

    /// Once per second, log the slowest phase if it exceeded 2 ms.
    fn report(&mut self, now: Instant) {
        if now - self.last_report < Duration::from_secs(1) {
            return;
        }
        self.last_report = now;
        if let Some((name, elapsed)) = self.phase.take()
            && elapsed > Duration::from_millis(3)
        {
            log::warn!("slow phase: {} took {} us", name, elapsed.as_micros());
        }
    }
}

/// The entry point: create the screens, then route, update and draw forever.
#[esp_rtos::main]
async fn main(_spawner: Spawner) -> ! {
    let Board {
        mut display,
        touch,
        imu,
        microphone,
        speaker,
        network,
        camera,
        backlight,
        log,
    } = Board::init();

    let mut screens = Screens {
        network: NetworkScreen::new(network),
        imu: ImuScreen::new(imu),
        microphone: MicrophoneScreen::new(microphone),
        speaker: SpeakerScreen::new(speaker),
        camera: CameraScreen::new(camera),
        settings: SettingsScreen::new(backlight),
        log: LogScreen::new(log),
    };
    let mut navigation = Navigation::new(touch);
    // One canvas the size of the content area, shared by all screens.
    let mut canvas = Canvas::new(layout::CONTENT_SIZE);

    let mut slowest = Slowest {
        phase: None,
        last_report: Instant::now(),
    };
    let mut active = ViewId::ALL[0];
    navigation::render(&mut display.surface(layout::NAV_AREA), active);
    screens.get_mut(active).enter();

    loop {
        let now = Instant::now();

        let started = Instant::now();
        let selected = navigation.poll(|event| screens.get_mut(active).handle_touch(event));
        slowest.record("poll touch", started.elapsed());
        if let Some(next) = selected
            && next != active
        {
            screens.get_mut(active).leave();
            active = next;
            screens.get_mut(active).enter();
            // Some screens draw on the surface directly; the panel no
            // longer shows what the canvas last showed.
            canvas.invalidate();
            navigation::render(&mut display.surface(layout::NAV_AREA), active);
            log::info!("Screen {:?}", active);
        }

        screens.update_all(now, &mut slowest);
        let started = Instant::now();
        screens
            .get_mut(active)
            .present(&mut canvas, &mut display.surface(layout::CONTENT_AREA));
        if active != ViewId::Camera {
            slowest.record("present", started.elapsed());
        }
        slowest.report(now);

        // DIAGNOSTIC (temporary): no sleep while the camera is live, as the
        // original camera application did.
        if active != ViewId::Camera {
            let started = Instant::now();
            Timer::after(LOOP_PERIOD).await;
            slowest.record("sleep", started.elapsed());
        }
    }
}
