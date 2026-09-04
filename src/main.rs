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
mod imu;
mod live_views;
mod logger;
mod memory;
mod models;
mod resources;
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

    let (cpu0_app_endpoint, cpu1_service_endpoint) = cross_core::split();

    // Partition hardware and communication endpoints into the two intended
    // architectural sides. These resource types make ownership visible but do
    // not try to prove which physical core is executing.
    let runtime_resources = resources::RuntimeResources {
        cpu0: resources::Cpu0Resources {
            display: screen::Resources {
                spi2: peripherals.SPI2,
                dma: peripherals.DMA_CH1,
                sck: peripherals.GPIO36,
                mosi: peripherals.GPIO37,
                dc: peripherals.GPIO35,
                cs: peripherals.GPIO3,
            },
            app: cpu0_app_endpoint,
        },
        cpu1: resources::Cpu1Resources {
            system_i2c: system_i2c::Resources {
                i2c0: peripherals.I2C0,
                sda: peripherals.GPIO12,
                scl: peripherals.GPIO11,
            },
            audio: audio::Resources {
                i2s0: peripherals.I2S0,
                dma: peripherals.DMA_CH0,
                mclk: peripherals.GPIO0,
                bclk: peripherals.GPIO34,
                word_select: peripherals.GPIO33,
                data_in: peripherals.GPIO14,
            },
            services: cpu1_service_endpoint,
        },
    };

    let resources::RuntimeResources { cpu0, cpu1 } = runtime_resources;
    let resources::Cpu0Resources {
        display,
        app: _cpu0_app_endpoint,
    } = cpu0;
    let resources::Cpu1Resources {
        system_i2c: system_i2c_resources,
        audio: audio_resources,
        services: cpu1_service_endpoint,
    } = cpu1;

    let mut delay = esp_hal::delay::Delay::new();
    let mut system_i2c = system_i2c::init(system_i2c_resources);

    let mut screen = screen::init(&mut system_i2c, display, &mut delay);

    audio::init_es7210(&mut system_i2c, &mut delay)
        .expect("Failed to initialize ES7210 microphone codec");

    info!("==========================================");
    info!(">>> M5Stack CoreS3 Lite Booting Up! <<<");
    info!("==========================================");

    memory::report("before CPU1 startup");

    info!("Starting CPU1 acquisition executor");

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
                // Reserved for future low-rate application/service commands.
                // Touch/audio/IMU keep their specialized cross-core data paths.
                let _cpu1_service_endpoint = cpu1_service_endpoint;

                spawner.spawn(
                    memory::cpu1_stack_monitor_task()
                        .expect("Failed to allocate CPU1 stack monitor task"),
                );

                let system_bus = system_i2c::into_async(system_i2c);

                spawner.spawn(
                    imu::capture_task(system_bus, imu::DEFAULT_CONFIG)
                        .expect("Failed to allocate CPU1 IMU task"),
                );

                spawner.spawn(
                    touch::capture_task(system_bus).expect("Failed to allocate CPU1 touch task"),
                );

                spawner.spawn(
                    audio::capture_task(audio_resources)
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
        ui.render_direct_view(&mut screen);

        heap_monitor.end_navigation();
        heap_monitor.poll(now);

        Timer::after(UI_IDLE_DELAY).await;
    }
}
