#![no_std]
#![no_main]
#![deny(unsafe_op_in_unsafe_fn)]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

#[cfg(any(
    feature = "imu-worldview",
    feature = "mic-waveform",
    feature = "speaker-synth",
    feature = "network-demo",
    feature = "camera-view",
    feature = "settings",
    feature = "log-view",
))]
mod app;
mod capabilities;
mod firmware;
mod platform;
mod support;
#[cfg(any(
    feature = "imu-worldview",
    feature = "mic-waveform",
    feature = "speaker-synth",
    feature = "network-demo",
    feature = "camera-view",
    feature = "settings",
    feature = "log-view",
))]
mod ui;

extern crate alloc;

#[cfg(any(
    feature = "imu-worldview",
    feature = "mic-waveform",
    feature = "speaker-synth",
    feature = "network-demo",
    feature = "camera-view",
    feature = "settings",
    feature = "log-view",
))]
use ::log::info;
use embassy_executor::Spawner;
#[cfg(any(
    feature = "imu-worldview",
    feature = "mic-waveform",
    feature = "speaker-synth",
    feature = "network-demo",
    feature = "camera-view",
    feature = "settings",
    feature = "log-view",
))]
use embassy_time::Instant;
use embassy_time::{Duration, Timer};
use esp_backtrace as _;

const UI_IDLE_DELAY: Duration = Duration::from_millis(5);

esp_bootloader_esp_idf::esp_app_desc!();

#[esp_rtos::main]
async fn main(_cpu0_spawner: Spawner) -> ! {
    let bootstrap = firmware::bootstrap();

    #[cfg(any(
        feature = "imu-worldview",
        feature = "mic-waveform",
        feature = "speaker-synth",
        feature = "network-demo",
        feature = "camera-view",
        feature = "settings",
        feature = "log-view",
    ))]
    {
        let mut bootstrap = bootstrap;
        let model = app::model::AppModel::new(app::model::AppModelInputs {
            #[cfg(feature = "network-demo")]
            network: bootstrap.inputs.network,
            #[cfg(feature = "imu-worldview")]
            imu: bootstrap.inputs.imu,
            #[cfg(feature = "mic-waveform")]
            audio: bootstrap.inputs.audio,
            #[cfg(feature = "speaker-synth")]
            playback: bootstrap.playback,
            #[cfg(feature = "settings")]
            brightness: bootstrap.brightness,
            #[cfg(feature = "log-view")]
            log: bootstrap.inputs.log,
        });

        #[cfg(feature = "touch")]
        let mut ui = ui::Ui::new(model, bootstrap.inputs.touch);
        #[cfg(not(feature = "touch"))]
        let mut ui = ui::Ui::new(model);

        let display = &mut bootstrap.display;
        let now = Instant::now();
        let mut heap_monitor = support::memory::HeapMonitor::new(now);
        heap_monitor.checkpoint("after model + UI construction");

        ui.render_initial(display);
        heap_monitor.checkpoint("after initial UI render");

        loop {
            let now = Instant::now();
            let transition = ui.prepare_frame(now);

            if let Some(transition) = transition {
                heap_monitor.begin_activity(transition.to.name());
                #[cfg(feature = "camera-view")]
                if transition.from == app::model::ViewId::Camera {
                    bootstrap.camera.pause();
                }
                ui.apply_navigation(transition, display);
            }

            ui.render(display);

            #[cfg(feature = "camera-view")]
            let camera_active =
                bootstrap.camera_ready && ui.presented_view() == app::model::ViewId::Camera;
            #[cfg(not(feature = "camera-view"))]
            let camera_active = false;

            #[cfg(feature = "camera-view")]
            if camera_active {
                if let Some(mut frame) = bootstrap.camera.begin_frame() {
                    ui.render_camera(display, &mut frame);
                    frame.finish();
                }
            }

            if let Some(transition) = transition {
                heap_monitor.end_activity();
                info!("View {:?} -> {:?}", transition.from, transition.to);
            }
            heap_monitor.poll(now);

            #[cfg(feature = "imu-worldview")]
            let imu_active = ui.presented_view() == app::model::ViewId::Imu;
            #[cfg(not(feature = "imu-worldview"))]
            let imu_active = false;

            if !camera_active && !imu_active {
                Timer::after(UI_IDLE_DELAY).await;
            }
        }
    }

    #[cfg(not(any(
        feature = "imu-worldview",
        feature = "mic-waveform",
        feature = "speaker-synth",
        feature = "network-demo",
        feature = "camera-view",
        feature = "settings",
        feature = "log-view",
    )))]
    {
        let _bootstrap = bootstrap;
        loop {
            Timer::after(UI_IDLE_DELAY).await;
        }
    }
}
