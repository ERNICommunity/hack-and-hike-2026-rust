//! The full Hack & Hike demo: eight screens for network, IMU, microphone,
//! speaker, camera, proximity (with ambient light), settings (backlight) and
//! the log.
//!
//! `main` starts the board, gives each screen the handles it owns, and runs
//! the loop. Each loop iteration does these steps:
//!
//! 1. Route the touch events.
//! 2. Update every screen.
//! 3. Draw the visible screen.
//! 4. Pause for 2 ms, unless the visible screen does not allow it.
//!
//! ```text
//! ┌────┬───────────────────────────┐
//! │ 📶 │                           │
//! │ 🧭 │   the visible screen      │
//! │ 🎤 │   (content area,          │
//! │ 🔊 │    276 x 240 pixels)      │
//! │ 📷 │                           │
//! │ ☀  │                           │
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
//! - `styles`: the widget styles that the KDL layout files refer to. KDL is
//!   a small document language. Each screen describes its static layout in
//!   a `.kdl` file.

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
    network::NetworkScreen, proximity::ProximityScreen, settings::SettingsScreen,
    speaker::SpeakerScreen,
};

// Writes the application descriptor the bootloader checks before starting
// the firmware. Every application needs this line exactly once.
esp_bootloader_esp_idf::esp_app_desc!();

/// Pause between loop iterations, unless the visible screen does not allow it
/// (see [`Screen::may_idle`]). It is short, so the IMU screen follows the
/// board without a visible delay. The `.await` lets other CPU0 tasks run.
const LOOP_PERIOD: Duration = Duration::from_millis(2);

/// Every screen, one field each. This is a struct and not an array, because
/// the screens are different types.
struct Screens {
    /// ESP-NOW pings and pongs, and the peers in range. ESP-NOW is
    /// Espressif's protocol for short Wi-Fi messages between boards.
    network: NetworkScreen,
    /// Roll, pitch and yaw, with a 3-D horizon and compass.
    imu: ImuScreen,
    /// The live waveform of both microphone channels.
    microphone: MicrophoneScreen,
    /// The melody and chime player. It keeps playing while another screen is
    /// visible.
    speaker: SpeakerScreen,
    /// The live camera preview.
    camera: CameraScreen,
    /// Proximity and ambient light, from one sensor.
    proximity: ProximityScreen,
    /// The display brightness slider.
    settings: SettingsScreen,
    /// The newest lines of the device log.
    log: LogScreen,
}

impl Screens {
    /// The screen for `id`, as a trait object. So the shell calls the same
    /// methods on every screen and does not need to know its type.
    fn get_mut(&mut self, id: ViewId) -> &mut dyn Screen {
        match id {
            ViewId::Network => &mut self.network,
            ViewId::Imu => &mut self.imu,
            ViewId::Microphone => &mut self.microphone,
            ViewId::Speaker => &mut self.speaker,
            ViewId::Camera => &mut self.camera,
            ViewId::Proximity => &mut self.proximity,
            ViewId::Settings => &mut self.settings,
            ViewId::Log => &mut self.log,
        }
    }

    /// Let every screen do its background work, visible or not. For example,
    /// the speaker screen writes audio and the network screen reads messages.
    fn update_all(&mut self, now: Instant) {
        for id in ViewId::ALL {
            self.get_mut(id).update(now);
        }
    }
}

/// The entry point: create the screens, then route touches, update and draw
/// in an endless loop.
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
        light,
        proximity,
    } = Board::init();

    let mut screens = Screens {
        network: NetworkScreen::new(network),
        imu: ImuScreen::new(imu),
        microphone: MicrophoneScreen::new(microphone),
        speaker: SpeakerScreen::new(speaker),
        camera: CameraScreen::new(camera),
        proximity: ProximityScreen::new(light, proximity),
        settings: SettingsScreen::new(backlight),
        log: LogScreen::new(log),
    };
    let mut navigation = Navigation::new(touch);
    // One canvas the size of the content area, shared by all screens.
    let mut canvas = Canvas::new(layout::CONTENT_SIZE);

    let mut active = ViewId::ALL[0];
    navigation::render(&mut display.surface(layout::NAV_AREA), active);
    screens.get_mut(active).enter();

    loop {
        let now = Instant::now();

        let selected = navigation.poll(|event| screens.get_mut(active).handle_touch(event));
        if let Some(next) = selected
            && next != active
        {
            screens.get_mut(active).leave();
            active = next;
            screens.get_mut(active).enter();
            // Some screens draw on the surface directly, without the canvas.
            // So the panel may not show what the canvas showed last.
            canvas.invalidate();
            navigation::render(&mut display.surface(layout::NAV_AREA), active);
            log::info!("Screen {:?}", active);
        }

        screens.update_all(now);
        screens
            .get_mut(active)
            .present(&mut canvas, &mut display.surface(layout::CONTENT_AREA));

        if screens.get_mut(active).may_idle() {
            Timer::after(LOOP_PERIOD).await;
        }
    }
}
