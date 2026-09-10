#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

mod app;
mod firmware;
mod platform;
mod services;
mod support;

extern crate alloc;

use app::{model as models, ui};
use app::{model::microphone as waveform, ui::theme};
use platform::{board, i2c as system_i2c};
use services::{audio, camera, display, imu, network, touch};
use services::network::protocol;
use support::memory::data_plane;
use support::{diagnostics, logging as logger, memory};

use ::log::info;
use embassy_executor::Spawner;
use embassy_time::{Duration, Instant, Timer};
use esp_backtrace as _;

const UI_IDLE_DELAY: Duration = Duration::from_millis(5);

esp_bootloader_esp_idf::esp_app_desc!();

#[esp_rtos::main]
async fn main(_cpu0_spawner: Spawner) -> ! {
    let firmware::Bootstrap {
        mut display,
        mut camera,
        camera_ready,
        inputs,
        brightness,
        playback,
    } = firmware::bootstrap();

    let firmware::AppInputs {
        touch,
        imu: imu_input,
        audio: audio_input,
        network: network_input,
        log: log_input,
    } = inputs;
    let model = models::AppModel::new(
        models::AppModelInputs {
            network: network_input,
            imu: imu_input,
            audio: audio_input,
            log: log_input,
        },
        brightness,
        playback,
    );
    let mut ui = ui::Ui::new(model, touch);

    let now = Instant::now();
    let mut heap_monitor = memory::HeapMonitor::new(now);
    heap_monitor.checkpoint("after model + UI construction");

    ui.render_initial(&mut display);
    heap_monitor.checkpoint("after initial UI render");

    loop {
        let now = Instant::now();
        let transition = ui.prepare_frame(now);

        if let Some(transition) = transition {
            heap_monitor.begin_activity(transition.to.name());
            if transition.from == models::ViewId::Camera {
                camera.pause();
            }
            ui.apply_navigation(transition, &mut display);
        }

        ui.render(&mut display);

        let camera_active = camera_ready && ui.presented_view() == models::ViewId::Camera;
        if camera_active {
            if let Some(mut frame) = camera.begin_frame() {
                ui.render_camera(&mut display, &mut frame);
                let _ = frame.finish();
            }
        }

        if let Some(transition) = transition {
            heap_monitor.end_activity();
            info!("View {:?} -> {:?}", transition.from, transition.to);
        }
        heap_monitor.poll(now);

        // Camera capture is frame-paced by the sensor, while the IMU view is
        // paced by fresh 100 Hz fusion snapshots plus the proven 40 MHz LCD path.
        // Do not insert an arbitrary CPU0 sleep for either high-rate view; other
        // screens retain the small idle delay to avoid unnecessary busy looping.
        let imu_active = ui.presented_view() == models::ViewId::Imu;
        if !camera_active && !imu_active {
            Timer::after(UI_IDLE_DELAY).await;
        }
    }
}
