#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

mod audio;
mod log;
mod screen;
mod system_i2c;

extern crate alloc;

use ::log::{error, info};
use alloc::rc::Rc;
use bt_hci::controller::ExternalController;
use embassy_executor::Spawner;
use esp_backtrace as _;
use esp_hal::clock::CpuClock;
use esp_hal::timer::timg::TimerGroup;
use esp_radio::ble::controller::BleConnector;
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter};
use slint::Model;
use trouble_host::prelude::*;

const CONNECTIONS_MAX: usize = 1;
const L2CAP_CHANNELS_MAX: usize = 1;

/// Synchronize the persistent Slint log model with the logger snapshot without
/// rebuilding every SharedString on every refresh.
///
/// The logger stores newest lines first, while the UI model stores oldest lines
/// first. Since new log entries are prepended and old entries are only evicted
/// from the tail, most updates can be represented as:
///   1. remove any evicted rows from the front of the model;
///   2. append only the genuinely new rows.
fn sync_log_model(model: &slint::VecModel<slint::SharedString>, logs: &str) {
    let old_count = model.row_count();

    // First population, or recovery after an unexpected discontinuity.
    if old_count == 0 {
        for line in logs.lines().rev() {
            model.push(slint::SharedString::from(line));
        }
        return;
    }

    let Some(old_newest) = model.row_data(old_count - 1) else {
        return;
    };

    // `logs` is newest -> oldest. Find the previous newest row in the new
    // snapshot. Everything before it is newly-added data. Verify the older
    // rows too, so an identical repeated log message does not create a false
    // match.
    let mut match_info = None;

    for (new_prefix_count, line) in logs.lines().enumerate() {
        if line != old_newest.as_str() {
            continue;
        }

        let mut overlap = 1usize;
        let mut old_index = old_count - 1;
        let mut valid = true;

        for older_line in logs.lines().skip(new_prefix_count + 1) {
            if old_index == 0 {
                break;
            }

            old_index -= 1;
            let Some(old_line) = model.row_data(old_index) else {
                valid = false;
                break;
            };

            if old_line.as_str() != older_line {
                valid = false;
                break;
            }

            overlap += 1;
        }

        if valid {
            match_info = Some((new_prefix_count, overlap));
            break;
        }
    }

    let Some((new_prefix_count, overlap)) = match_info else {
        // This should be rare (for example if the logger rolled over by more
        // than the entire previous model between UI refreshes). Rebuild once
        // to recover, rather than carrying a stale model forward.
        model.clear();
        for line in logs.lines().rev() {
            model.push(slint::SharedString::from(line));
        }
        return;
    };

    // Drop only rows that the logger evicted from its fixed-size ring buffer.
    for _ in 0..old_count.saturating_sub(overlap) {
        model.remove(0);
    }

    // The new prefix is newest -> oldest, but rows are appended to the Slint
    // model oldest -> newest. Usually this loop executes exactly once.
    for index in (0..new_prefix_count).rev() {
        if let Some(line) = logs.lines().nth(index) {
            model.push(slint::SharedString::from(line));
        }
    }
}

// This creates a default app-descriptor required by the esp-idf bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html#application-description>
esp_bootloader_esp_idf::esp_app_desc!();

slint::include_modules!();

struct McuPlatform {
    window: Rc<MinimalSoftwareWindow>,
}

impl Platform for McuPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.window.clone())
    }

    fn duration_since_start(&self) -> core::time::Duration {
        core::time::Duration::from_millis(embassy_time::Instant::now().as_millis())
    }
}

#[allow(
    clippy::large_stack_frames,
    reason = "it's not unusual to allocate larger buffers etc. in main"
)]
#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    // Reclaim bootloader
    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 73744);
    esp_alloc::heap_allocator!(size: 128 * 1024);

    log::init(::log::LevelFilter::Info);
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let sw_interrupt =
        esp_hal::interrupt::software::SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    esp_rtos::start(timg0.timer0, sw_interrupt.software_interrupt0);

    // Hardware blocking delay driver
    let mut screen_delay = esp_hal::delay::Delay::new();

    // Initialize the internal 400 kHz I2C bus once. Touch, PMIC, ES7210,
    // and later the IMU can all obtain lightweight device handles to it.
    let system_i2c = system_i2c::init(peripherals.I2C0, peripherals.GPIO12, peripherals.GPIO11);

    // Initialize display + touch using one handle to the shared system bus.
    let mut touch = screen::init(
        system_i2c::device(system_i2c),
        peripherals.SPI2,
        peripherals.GPIO36,
        peripherals.GPIO37,
        peripherals.GPIO35,
        peripherals.GPIO3,
        &mut screen_delay,
    );

    // Power and configure the ES7210 microphone codec over the same I2C bus.
    let mut codec_i2c = system_i2c::device(system_i2c);
    audio::init_es7210(&mut codec_i2c, &mut screen_delay)
        .expect("Failed to initialize ES7210 microphone codec");
    drop(codec_i2c);

    // Broadcast immediately on CPU start
    info!("==========================================");
    info!(">>> M5Stack CoreS3 Lite Booting Up! <<<");
    info!("==========================================");

    // The following pins are used to bootstrap the chip. They are available
    // for use, but check the datasheet of the module for more information on them.
    // - GPIO0
    // - GPIO3
    // - GPIO45
    // - GPIO46
    // These GPIO pins are in use by some feature of the module and should not be used.
    let _ = peripherals.GPIO27;
    let _ = peripherals.GPIO28;
    let _ = peripherals.GPIO29;
    let _ = peripherals.GPIO30;
    let _ = peripherals.GPIO31;
    let _ = peripherals.GPIO32;
    // let _ = peripherals.GPIO35;
    // let _ = peripherals.GPIO36;
    // let _ = peripherals.GPIO37;

    let (mut _wifi_controller, _interfaces) =
        esp_radio::wifi::new(peripherals.WIFI, Default::default())
            .expect("Failed to initialize Wi-Fi controller");
    // find more examples https://github.com/embassy-rs/trouble/tree/main/examples/esp32
    let transport = BleConnector::new(peripherals.BT, Default::default()).unwrap();
    let ble_controller = ExternalController::<_, 1>::new(transport);
    let mut resources: HostResources<DefaultPacketPool, CONNECTIONS_MAX, L2CAP_CHANNELS_MAX> =
        HostResources::new();
    let _ble_stack = trouble_host::new(ble_controller, &mut resources);

    info!("Spawning tasks");

    // Continuously capture the two onboard microphones with circular I2S DMA.
    // GPIO0=MCLK, GPIO34=BCLK, GPIO33=WS/LRCLK, GPIO14=ES7210 data out.
    if let Ok(task) = audio::capture_task(
        peripherals.I2S0,
        peripherals.DMA_CH0,
        peripherals.GPIO0,
        peripherals.GPIO34,
        peripherals.GPIO33,
        peripherals.GPIO14,
    )
    .map_err(|err| error!("Audio Capture Spawn Error {}", err))
    {
        spawner.spawn(task);
    }

    let window = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    window.set_size(slint::PhysicalSize::new(320, 240));

    slint::platform::set_platform(alloc::boxed::Box::new(McuPlatform {
        window: window.clone(),
    }))
    .expect("Failed to initialize Slint platform");

    let ui = AppWindow::new().unwrap();
    ui.show().unwrap();

    // Keep one VecModel for the lifetime of the UI. Updating the model in
    // place lets Slint preserve existing SharedStrings and the Vec capacity
    // instead of allocating a complete replacement model on every log tick.
    let log_model = Rc::new(slint::VecModel::<slint::SharedString>::default());
    ui.set_log_lines(log_model.clone().into());

    // Seed the model with the startup logs already collected above.
    log::with_logs(|logs| sync_log_model(&log_model, logs));

    let mut count = 0;
    let mut last_heartbeat = embassy_time::Instant::now();

    loop {
        let now = embassy_time::Instant::now();
        if now - last_heartbeat >= embassy_time::Duration::from_millis(350) && count < 100 {
            last_heartbeat = now;

            info!("Heartbeat count: {}", count);
            count += 1;

            // Incrementally synchronize the persistent model. Existing rows
            // are retained; only evicted rows are removed and new rows allocate
            // a new SharedString.
            log::with_logs(|logs| sync_log_model(&log_model, logs));
        }

        // Forward FT6336 touch input to Slint before updating/rendering the UI.
        touch.poll(&window);

        // Re-render UI & update animation ticks
        slint::platform::update_timers_and_animations();
        screen::render_slint_window(&window);

        embassy_time::Timer::after(embassy_time::Duration::from_millis(5)).await;
    }

    // for inspiration have a look at the examples at https://github.com/esp-rs/esp-hal/tree/esp-hal-v1.1.0/examples
}
