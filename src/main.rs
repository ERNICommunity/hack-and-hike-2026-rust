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
mod camera;
mod data_plane;
mod diagnostics;
mod display;
mod display_control;
mod imu;
mod logger;
mod memory;
mod models;
mod network;
mod protocol;
mod resources;
mod service_inputs;
mod system_i2c;
mod theme;
mod touch;
mod ui;
mod waveform;

extern crate alloc;

use ::log::{info, warn};
use embassy_executor::Spawner;
use embassy_time::{Duration, Instant, Timer};
use esp_backtrace as _;
use esp_hal::{clock::CpuClock, system::Stack, timer::timg::TimerGroup};
use static_cell::StaticCell;

const CPU1_STACK_SIZE: usize = 16 * 1024;
const UI_IDLE_DELAY: Duration = Duration::from_millis(5);
const CAMERA_IDLE_DELAY: Duration = Duration::from_millis(1);

static CPU1_STACK: StaticCell<Stack<CPU1_STACK_SIZE>> = StaticCell::new();
static CPU1_EXECUTOR: StaticCell<esp_rtos::embassy::Executor> = StaticCell::new();

esp_bootloader_esp_idf::esp_app_desc!();

#[esp_rtos::main]
async fn main(_cpu0_spawner: Spawner) -> ! {
    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 73744);
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

    let runtime_resources = resources::RuntimeResources {
        cpu0: resources::Cpu0Resources {
            display: display::Resources {
                spi2: peripherals.SPI2,
                dma: peripherals.DMA_CH1,
                sck: peripherals.GPIO36,
                mosi: peripherals.GPIO37,
                dc: peripherals.GPIO35,
                cs: peripherals.GPIO3,
            },
            camera: camera::Resources {
                lcd_cam: peripherals.LCD_CAM,
                dma: peripherals.DMA_CH2,
                pclk: peripherals.GPIO45,
                vsync: peripherals.GPIO46,
                href: peripherals.GPIO38,
                d0: peripherals.GPIO39,
                d1: peripherals.GPIO40,
                d2: peripherals.GPIO41,
                d3: peripherals.GPIO42,
                d4: peripherals.GPIO15,
                d5: peripherals.GPIO16,
                d6: peripherals.GPIO48,
                d7: peripherals.GPIO47,
            },
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
                data_out: peripherals.GPIO13,
            },
            network: network::Resources {
                wifi: peripherals.WIFI,
            },
        },
    };

    let resources::RuntimeResources { cpu0, cpu1 } = runtime_resources;
    let resources::Cpu0Resources {
        display: display_resources,
        camera: camera_resources,
    } = cpu0;
    let resources::Cpu1Resources {
        system_i2c: mut system_i2c_resources,
        audio: audio_resources,
        network: network_resources,
    } = cpu1;

    let mut delay = esp_hal::delay::Delay::new();

    // Board bootstrap initially borrows the runtime I2C resources only long
    // enough to configure the PMIC and AW9523. The driver is then dropped so a
    // fresh camera SCCB owner can take I2C0 + GPIO12/GPIO11.
    let mut bootstrap_i2c = system_i2c::init(system_i2c_resources.reborrow());
    board::power::enable_lcd_backlight(&mut bootstrap_i2c);
    board::io_expander::reset_display_and_touch(&mut bootstrap_i2c, &mut delay);

    // CoreS3/CoreS3-Lite power the GC0308 and its onboard 20 MHz oscillator
    // from AXP2101 ALDO3. Camera bring-up remains optional: a camera fault must
    // not prevent the rest of the device from booting.
    let camera_powered_and_reset = match board::power::enable_camera(&mut bootstrap_i2c) {
        Ok(()) => {
            delay.delay_millis(10u32);
            match board::io_expander::reset_camera(&mut bootstrap_i2c, &mut delay) {
                Ok(()) => {
                    match board::power::camera_power_registers(&mut bootstrap_i2c) {
                        Ok((enable, voltage)) => info!(
                            "Camera PMIC readback: AXP2101[0x90]=0x{:02x} AXP2101[0x94]=0x{:02x}",
                            enable, voltage
                        ),
                        Err(error) => warn!("Camera PMIC readback failed: {:?}", error),
                    }
                    match board::io_expander::camera_reset_registers(&mut bootstrap_i2c) {
                        Ok((output, direction, mode)) => info!(
                            "Camera reset readback: AW9523 P1 out=0x{:02x} dir=0x{:02x} mode=0x{:02x}",
                            output, direction, mode
                        ),
                        Err(error) => warn!("Camera reset readback failed: {:?}", error),
                    }
                    true
                }
                Err(error) => {
                    warn!("Camera disabled: GC0308 reset failed: {:?}", error);
                    false
                }
            }
        }
        Err(error) => {
            warn!("Camera disabled: failed to enable ALDO3 rail: {:?}", error);
            false
        }
    };

    // M5Stack's CoreS3 camera fix explicitly releases the shared internal-I2C
    // owner before camera initialization. Do the same: destroy the bootstrap
    // driver, create a fresh 100 kHz hardware SCCB owner, program/read GC0308,
    // then destroy that owner before creating the persistent runtime bus.
    drop(bootstrap_i2c);
    let camera_ready = if camera_powered_and_reset {
        let mut camera_sccb = system_i2c::init_camera_sccb(system_i2c_resources.reborrow());
        let ready = match camera::init_sensor(&mut camera_sccb, &mut delay) {
            Ok(pid) if pid == camera::EXPECTED_SENSOR_PID => {
                info!("GC0308 camera ready (PID=0x{:02x})", pid);
                true
            }
            Ok(pid) => {
                warn!(
                    "Camera disabled: unexpected GC0308 PID 0x{:02x} (expected 0x{:02x})",
                    pid,
                    camera::EXPECTED_SENSOR_PID
                );
                false
            }
            Err(error) => {
                warn!("Camera disabled: GC0308 hardware SCCB failed: {:?}", error);
                false
            }
        };
        drop(camera_sccb);
        ready
    } else {
        false
    };

    // Camera SCCB is gone now. Construct the final 400 kHz system-I2C owner;
    // this exact driver is later moved to CPU1 and converted to async mode.
    let mut system_i2c = system_i2c::init(system_i2c_resources);

    let mut display = display::init(display_resources, &mut delay);
    let mut camera = camera::init(camera_resources);

    audio::init_es7210(&mut system_i2c).expect("Failed to initialize ES7210 microphone codec");
    audio::init_aw88298(&mut system_i2c, &mut delay)
        .expect("Failed to initialize AW88298 speaker amplifier");

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
                spawner.spawn(
                    memory::cpu1_stack_monitor_task()
                        .expect("Failed to allocate CPU1 stack monitor task"),
                );
                network::start(&spawner, network_resources, network::DEFAULT_CONFIG);

                let system_bus = system_i2c::into_async(system_i2c);
                spawner.spawn(
                    display_control::task(system_bus)
                        .expect("Failed to allocate CPU1 display-control task"),
                );
                spawner.spawn(
                    imu::capture_task(system_bus, imu::DEFAULT_CONFIG)
                        .expect("Failed to allocate CPU1 IMU task"),
                );
                spawner.spawn(
                    touch::capture_task(system_bus).expect("Failed to allocate CPU1 touch task"),
                );
                spawner.spawn(
                    audio::capture_task(audio_resources, spawner)
                        .expect("Failed to allocate CPU1 audio task"),
                );
            });
        },
    );

    let service_inputs::Cpu0Inputs {
        touch,
        imu: imu_input,
        audio: audio_input,
        network: network_input,
    } = service_inputs::Cpu0Inputs::from_static_services();
    let brightness = display_control::BrightnessControl::from_static_service();
    let playback = audio::PlaybackControl::from_static_service();
    let model = models::AppModel::new(
        models::AppModelInputs {
            network: network_input,
            imu: imu_input,
            audio: audio_input,
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
            ui.apply_navigation(transition, &mut display);
        }

        ui.render(&mut display);

        let camera_active = camera_ready && ui.presented_view() == models::ViewId::Camera;
        if camera_active {
            if let Some(frame) = camera.capture() {
                ui.render_camera(&mut display, &frame);
            }
        }

        if let Some(transition) = transition {
            heap_monitor.end_activity();
            info!("View {:?} -> {:?}", transition.from, transition.to);
        }
        heap_monitor.poll(now);

        Timer::after(if camera_active {
            CAMERA_IDLE_DELAY
        } else {
            UI_IDLE_DELAY
        })
        .await;
    }
}
