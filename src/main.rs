#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

mod audio;
mod board;
pub mod cross_core;
mod data_plane;
mod diagnostics;
mod logger;
mod memory;
mod models;
mod runtime_architecture;
mod screen;
mod system_i2c;
mod theme;
mod touch;
mod ui;
mod waveform;

extern crate alloc;

use ::log::info;
use embassy_executor::Spawner;
use embassy_time::{Duration, Instant, Timer};
use esp_backtrace as _;
use esp_hal::{clock::CpuClock, system::Stack, timer::timg::TimerGroup};
use static_cell::StaticCell;

const CPU1_STACK_SIZE: usize = 16 * 1024;
const UI_IDLE_DELAY: Duration = Duration::from_millis(5);

static CPU1_STACK: StaticCell<Stack<CPU1_STACK_SIZE>> = StaticCell::new();
static CPU1_EXECUTOR: StaticCell<esp_rtos::embassy::Executor> = StaticCell::new();

esp_bootloader_esp_idf::esp_app_desc!();

#[esp_rtos::main]
async fn main(_cpu0_spawner: Spawner) -> ! {
    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 73744);
    // Internal DRAM is shared between statics/global heap and the CPU0 stack.
    // Keep the global heap deliberately smaller now that bulk application data
    // lives in PSRAM. The previous 128 KiB reservation left only ~20 KiB for
    // the ProCpu stack and Slint's software renderer could cross its guard.
    //
    // 104 KiB returns 24 KiB to the linker-defined CPU0 stack while retaining
    // substantial internal-heap headroom for Slint/radio/runtime objects.
    esp_alloc::heap_allocator!(size: 104 * 1024);

    logger::init(::log::LevelFilter::Info);
    memory::init_cpu0_stack_watermark();

    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    memory::enable_psram(peripherals.PSRAM);
    logger::enable_psram_history();
    memory::report("PSRAM/data-plane ready");

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let sw_interrupt =
        esp_hal::interrupt::software::SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    esp_rtos::start(timg0.timer0, sw_interrupt.software_interrupt0);

    // Express the runtime ownership split in concrete Rust types before
    // constructing services. These are ordinary resource containers, not CPU
    // capability tokens: the types document and shape the architecture while
    // the bootstrap code remains responsible for placing them on the right core.
    let runtime_resources = runtime_architecture::RuntimeArchitectureResources {
        cpu0_application_presentation_and_display:
            runtime_architecture::Cpu0ApplicationPresentationAndDisplayResources {
                display_io: runtime_architecture::Cpu0DisplayIoResources {
                    spi2: peripherals.SPI2,
                    display_dma: peripherals.DMA_CH1,
                    display_sck: peripherals.GPIO36,
                    display_mosi: peripherals.GPIO37,
                    display_dc: peripherals.GPIO35,
                    display_cs: peripherals.GPIO3,
                },
            },
        cpu1_non_display_peripheral_services:
            runtime_architecture::Cpu1NonDisplayPeripheralServicesResources {
                runtime_system_i2c: runtime_architecture::Cpu1RuntimeSystemI2cResources {
                    i2c0: peripherals.I2C0,
                    system_i2c_sda: peripherals.GPIO12,
                    system_i2c_scl: peripherals.GPIO11,
                },
                audio_acquisition: runtime_architecture::Cpu1AudioAcquisitionResources {
                    i2s0: peripherals.I2S0,
                    audio_dma: peripherals.DMA_CH0,
                    microphone_mclk: peripherals.GPIO0,
                    microphone_bclk: peripherals.GPIO34,
                    microphone_word_select: peripherals.GPIO33,
                    microphone_data_in: peripherals.GPIO14,
                },
            },
    };

    let runtime_architecture::RuntimeArchitectureResources {
        cpu0_application_presentation_and_display,
        cpu1_non_display_peripheral_services,
    } = runtime_resources;

    let runtime_architecture::Cpu0ApplicationPresentationAndDisplayResources { display_io } =
        cpu0_application_presentation_and_display;
    let runtime_architecture::Cpu1NonDisplayPeripheralServicesResources {
        runtime_system_i2c,
        audio_acquisition,
    } = cpu1_non_display_peripheral_services;

    let runtime_architecture::Cpu1RuntimeSystemI2cResources {
        i2c0,
        system_i2c_sda,
        system_i2c_scl,
    } = runtime_system_i2c;

    let mut delay = esp_hal::delay::Delay::new();
    let mut system_i2c = system_i2c::init(i2c0, system_i2c_sda, system_i2c_scl);

    let runtime_architecture::Cpu0DisplayIoResources {
        spi2,
        display_dma,
        display_sck,
        display_mosi,
        display_dc,
        display_cs,
    } = display_io;

    let mut screen = screen::init(
        &mut system_i2c,
        spi2,
        display_dma,
        display_sck,
        display_mosi,
        display_dc,
        display_cs,
        &mut delay,
    );

    audio::init_es7210(&mut system_i2c, &mut delay)
        .expect("Failed to initialize ES7210 microphone codec");

    info!("==========================================");
    info!(">>> M5Stack CoreS3 Lite Booting Up! <<<");
    info!("==========================================");

    memory::report("before CPU1 startup");

    info!("Starting CPU1 acquisition executor");

    let (_cpu0_application_cross_core_endpoint, cpu1_peripheral_services_cross_core_endpoint) =
        cross_core::split_application_and_peripheral_service_endpoints();

    let cpu1_stack = CPU1_STACK.init(Stack::new());
    memory::register_cpu1_stack(&mut *cpu1_stack);
    esp_rtos::start_second_core(
        peripherals.CPU_CTRL,
        sw_interrupt.software_interrupt1,
        cpu1_stack,
        move || {
            memory::init_cpu1_stack_watermark();
            let executor = CPU1_EXECUTOR.init(esp_rtos::embassy::Executor::new());

            executor.run(move |spawner| {
                // This typed endpoint belongs with the CPU1 peripheral-service
                // side. It is not used yet; future IMU/ESP-NOW service control
                // should use it instead of growing a generic sample queue.
                let _cpu1_peripheral_services_cross_core_endpoint =
                    cpu1_peripheral_services_cross_core_endpoint;

                spawner.spawn(
                    memory::cpu1_stack_monitor_task()
                        .expect("Failed to allocate CPU1 stack monitor task"),
                );

                let system_bus = system_i2c::into_async(system_i2c);

                spawner.spawn(
                    touch::capture_task(system_bus).expect("Failed to allocate CPU1 touch task"),
                );

                let runtime_architecture::Cpu1AudioAcquisitionResources {
                    i2s0,
                    audio_dma,
                    microphone_mclk,
                    microphone_bclk,
                    microphone_word_select,
                    microphone_data_in,
                } = audio_acquisition;

                spawner.spawn(
                    audio::capture_task(
                        i2s0,
                        audio_dma,
                        microphone_mclk,
                        microphone_bclk,
                        microphone_word_select,
                        microphone_data_in,
                    )
                    .expect("Failed to allocate CPU1 audio task"),
                );
            });
        },
    );

    let model = models::AppModel::new();
    let mut ui = ui::Ui::new(model);

    let now = Instant::now();
    let mut heap_monitor = memory::HeapMonitor::new(now);
    heap_monitor.checkpoint("after model + UI construction");

    // This is intentionally stack-heavy because it drives Slint's software
    // renderer through every persistent page. The memory report above now
    // includes both CPU0 stack size and current headroom.
    ui.prewarm_navigation(&mut screen);
    heap_monitor.checkpoint("after navigation prewarm");

    loop {
        let now = Instant::now();

        if let Some(change) = ui.prepare_frame(now) {
            // Capture allocator counters before even setting the Slint view
            // property, then measure again after the resulting render.
            heap_monitor.begin_navigation(change.to.as_i32());
            ui.apply_navigation(change);
            info!("View {:?} -> {:?}", change.from, change.to);
        }

        let slint_redrawn = screen.render_slint_window(ui.window());
        ui.note_slint_redraw(slint_redrawn);

        if let Some(frame) = ui.take_waveform_frame() {
            screen.render_waveform(&frame);
        }

        heap_monitor.end_navigation();
        heap_monitor.poll(now);

        Timer::after(UI_IDLE_DELAY).await;
    }
}
