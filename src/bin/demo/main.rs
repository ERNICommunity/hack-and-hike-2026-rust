//! Full Hack & Hike demo: one screen per capability plus settings and a log.
//!
//! `main` brings up the board, hands each screen the handles it owns, and
//! runs the loop: route touches, update every screen, draw the visible one.

#![no_std]
#![no_main]

mod layout;
mod navigation;
mod screens;
mod styles;

use embassy_executor::Spawner;
use embassy_time::{Duration, Instant, Timer};
use hack_and_hike::{Board, ui::gui::GuiSurface};

use navigation::{Navigation, ViewId};
use screens::{
    Screen, camera::CameraScreen, imu::ImuScreen, log::LogScreen, microphone::MicrophoneScreen,
    network::NetworkScreen, settings::SettingsScreen, speaker::SpeakerScreen,
};

esp_bootloader_esp_idf::esp_app_desc!();

/// Pause between loop iterations. Short enough for the camera and the IMU
/// screen to feel live, long enough to let CPU0 tasks run.
const LOOP_PERIOD: Duration = Duration::from_millis(2);

struct Screens {
    network: NetworkScreen,
    imu: ImuScreen,
    microphone: MicrophoneScreen,
    speaker: SpeakerScreen,
    camera: CameraScreen,
    settings: SettingsScreen,
    log: LogScreen,
}

impl Screens {
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

    fn update_all(&mut self, now: Instant) {
        for id in ViewId::ALL {
            self.get_mut(id).update(now);
        }
    }
}

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
    let mut gui = GuiSurface::new(layout::CONTENT_WIDTH, layout::CONTENT_HEIGHT);

    let mut active = ViewId::ALL[0];
    navigation::render(&mut display.surface(layout::NAV_REGION), active);
    screens.get_mut(active).enter();

    loop {
        let now = Instant::now();

        let selected = navigation.poll(|pointer| screens.get_mut(active).handle_pointer(pointer));
        if let Some(next) = selected
            && next != active
        {
            screens.get_mut(active).leave();
            active = next;
            screens.get_mut(active).enter();
            navigation::render(&mut display.surface(layout::NAV_REGION), active);
            log::info!("Screen {:?}", active);
        }

        screens.update_all(now);
        screens
            .get_mut(active)
            .present(&mut gui, &mut display.surface(layout::CONTENT_REGION));

        Timer::after(LOOP_PERIOD).await;
    }
}
