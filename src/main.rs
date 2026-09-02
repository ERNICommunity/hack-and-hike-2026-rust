#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

mod audio;
mod logger;
mod screen;
mod system_i2c;
mod touch;
mod ui;

extern crate alloc;

use ::log::info;
use bt_hci::controller::ExternalController;
use embassy_executor::Spawner;
use embassy_time::{Duration, Instant, Timer};
use esp_backtrace as _;
use esp_hal::{clock::CpuClock, system::Stack, timer::timg::TimerGroup};
use esp_radio::ble::controller::BleConnector;
use static_cell::StaticCell;
use trouble_host::prelude::*;

const CONNECTIONS_MAX: usize = 1;
const L2CAP_CHANNELS_MAX: usize = 1;
const CPU1_STACK_SIZE: usize = 16 * 1024;
const UI_IDLE_DELAY: Duration = Duration::from_millis(5);

static CPU1_STACK: StaticCell<Stack<CPU1_STACK_SIZE>> = StaticCell::new();
static CPU1_EXECUTOR: StaticCell<esp_rtos::embassy::Executor> = StaticCell::new();

// This creates the app descriptor required by the esp-idf bootloader.
esp_bootloader_esp_idf::esp_app_desc!();

#[allow(
    clippy::large_stack_frames,
    reason = "main owns long-lived board/radio state and the CPU0 presentation runtime"
)]
#[esp_rtos::main]
async fn main(_cpu0_spawner: Spawner) -> ! {
    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 73744);
    esp_alloc::heap_allocator!(size: 128 * 1024);

    logger::init(::log::LevelFilter::Info);

    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let sw_interrupt =
        esp_hal::interrupt::software::SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);

    // start_second_core() requires the CPU0 RTOS scheduler to be running first.
    esp_rtos::start(timg0.timer0, sw_interrupt.software_interrupt0);

    let mut delay = esp_hal::delay::Delay::new();
    let system_bus = system_i2c::init(peripherals.I2C0, peripherals.GPIO12, peripherals.GPIO11);

    // One-time board/display/audio initialization shares the same physical I2C
    // mutex later used by CPU1 touch acquisition. Screen retains only SPI2 + DMA1.
    let mut screen = {
        let mut i2c = system_bus.lock().await;

        let screen = screen::init(
            &mut *i2c,
            peripherals.SPI2,
            peripherals.DMA_CH1,
            peripherals.GPIO36,
            peripherals.GPIO37,
            peripherals.GPIO35,
            peripherals.GPIO3,
            &mut delay,
        );

        audio::init_es7210(&mut *i2c, &mut delay)
            .expect("Failed to initialize ES7210 microphone codec");

        screen
    };

    info!("==========================================");
    info!(">>> M5Stack CoreS3 Lite Booting Up! <<<");
    info!("==========================================");

    // Keep radio/network initialization unchanged; these resources stay alive
    // for the upcoming networking work.
    let (_wifi_controller, _interfaces) =
        esp_radio::wifi::new(peripherals.WIFI, Default::default())
            .expect("Failed to initialize Wi-Fi controller");

    let transport = BleConnector::new(peripherals.BT, Default::default()).unwrap();
    let ble_controller = ExternalController::<_, 1>::new(transport);
    let mut ble_resources: HostResources<DefaultPacketPool, CONNECTIONS_MAX, L2CAP_CHANNELS_MAX> =
        HostResources::new();
    let _ble_stack = trouble_host::new(ble_controller, &mut ble_resources);

    info!("Starting CPU1 acquisition executor");

    let cpu1_stack = CPU1_STACK.init(Stack::new());
    esp_rtos::start_second_core(
        peripherals.CPU_CTRL,
        sw_interrupt.software_interrupt1,
        cpu1_stack,
        move || {
            let executor = CPU1_EXECUTOR.init(esp_rtos::embassy::Executor::new());

            executor.run(move |spawner| {
                spawner.spawn(
                    touch::capture_task(system_bus).expect("Failed to allocate CPU1 touch task"),
                );

                spawner.spawn(
                    audio::capture_task(
                        peripherals.I2S0,
                        peripherals.DMA_CH0,
                        peripherals.GPIO0,
                        peripherals.GPIO34,
                        peripherals.GPIO33,
                        peripherals.GPIO14,
                    )
                    .expect("Failed to allocate CPU1 audio task"),
                );
            });
        },
    );

    // Everything from here down is CPU0-only presentation state.
    let mut ui = ui::Ui::new();

    loop {
        ui.update(Instant::now());
        screen.render_slint_window(ui.window());
        Timer::after(UI_IDLE_DELAY).await;
    }
}
