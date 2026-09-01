#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

mod log;
mod screen;

extern crate alloc;

use ::log::info;
use alloc::rc::Rc;
use bt_hci::controller::ExternalController;
use embassy_executor::Spawner;
use esp_backtrace as _;
use esp_hal::clock::CpuClock;
use esp_hal::timer::timg::TimerGroup;
use esp_radio::ble::controller::BleConnector;
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter};
use trouble_host::prelude::*;

const CONNECTIONS_MAX: usize = 1;
const L2CAP_CHANNELS_MAX: usize = 1;

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

    // Initialize the screen console
    let mut touch = screen::init(
        peripherals.I2C0,
        peripherals.SPI2,
        peripherals.GPIO12,
        peripherals.GPIO11,
        peripherals.GPIO36,
        peripherals.GPIO37,
        peripherals.GPIO35,
        peripherals.GPIO3,
        &mut screen_delay,
    );

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
    let _ = peripherals.GPIO33;
    let _ = peripherals.GPIO34;
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

    // TODO: Spawn some tasks
    let _ = spawner;

    let window = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    window.set_size(slint::PhysicalSize::new(320, 240));

    slint::platform::set_platform(alloc::boxed::Box::new(McuPlatform {
        window: window.clone(),
    }))
    .expect("Failed to initialize Slint platform");

    let ui = AppWindow::new().unwrap();
    ui.show().unwrap();

    let mut count = 0;
    let mut last_heartbeat = embassy_time::Instant::now();

    loop {
        let now = embassy_time::Instant::now();
        if now - last_heartbeat >= embassy_time::Duration::from_millis(50) && count < 100 {
            last_heartbeat = now;

            info!("Heartbeat count: {}", count);
            count += 1;

            // Fetch logs and update Slint string property
            log::with_logs(|logs| {
                let mut lines: alloc::vec::Vec<slint::SharedString> = logs
                    .lines()
                    .map(|line| slint::SharedString::from(line))
                    .collect();
                lines.reverse();
                let model = alloc::rc::Rc::new(slint::VecModel::from(lines));
                ui.set_log_lines(model.into());
            });
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
