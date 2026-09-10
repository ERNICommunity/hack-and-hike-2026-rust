//! CoreS3 Lite bootstrap and concrete firmware composition.
//!
//! The ordering in this module is hardware policy: board setup temporarily owns
//! the shared I2C peripheral at 400 kHz, camera SCCB temporarily recreates it at
//! 100 kHz, and only then is the final 400 kHz runtime owner constructed and
//! moved to CPU1. Keep those ownership transitions explicit and ordered.

use ::log::{info, warn};
use esp_hal::{clock::CpuClock, delay::Delay, timer::timg::TimerGroup};

use crate::{
    firmware::{cpu1, resources},
    platform::{board, i2c as system_i2c},
    services::{audio, camera, display, imu, network, touch},
    support::{logging as logger, memory},
};

pub(crate) struct AppInputs {
    pub(crate) touch: touch::Input,
    pub(crate) imu: imu::Input,
    pub(crate) audio: audio::Input,
    pub(crate) network: network::Input,
    pub(crate) log: logger::Input,
}

pub(crate) struct Bootstrap {
    pub(crate) display: display::Display,
    pub(crate) camera: camera::Camera,
    pub(crate) camera_ready: bool,
    pub(crate) inputs: AppInputs,
    pub(crate) brightness: display::BrightnessControl,
    pub(crate) playback: audio::PlaybackControl,
}

fn power_and_reset_camera<I2C>(i2c: &mut I2C, delay: &mut Delay) -> bool
where
    I2C: embedded_hal::i2c::I2c,
    I2C::Error: core::fmt::Debug,
{
    if let Err(error) = board::power::enable_camera(i2c) {
        warn!("Camera disabled: failed to enable ALDO3 rail: {:?}", error);
        return false;
    }

    delay.delay_millis(10u32);
    if let Err(error) = board::io_expander::reset_camera(i2c, delay) {
        warn!("Camera disabled: GC0308 reset failed: {:?}", error);
        return false;
    }

    match board::power::camera_power_registers(i2c) {
        Ok((enable, voltage)) => info!(
            "Camera PMIC readback: AXP2101[0x90]=0x{:02x} AXP2101[0x94]=0x{:02x}",
            enable, voltage
        ),
        Err(error) => warn!("Camera PMIC readback failed: {:?}", error),
    }
    match board::io_expander::camera_reset_registers(i2c) {
        Ok((output, direction, mode)) => info!(
            "Camera reset readback: AW9523 P1 out=0x{:02x} dir=0x{:02x} mode=0x{:02x}",
            output, direction, mode
        ),
        Err(error) => warn!("Camera reset readback failed: {:?}", error),
    }

    true
}

fn initialize_camera_sensor<I2C>(i2c: &mut I2C, delay: &mut Delay) -> bool
where
    I2C: embedded_hal::i2c::I2c,
    I2C::Error: core::fmt::Debug,
{
    match camera::init_sensor(i2c, delay) {
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
    }
}

pub(crate) fn bootstrap() -> Bootstrap {
    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 73744);
    esp_alloc::heap_allocator!(size: 104 * 1024);

    logger::init(::log::LevelFilter::Info);
    memory::init_cpu0_stack_watermark();

    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    memory::enable_psram(peripherals.PSRAM);
    let log_input = logger::enable_psram_history();
    memory::report("PSRAM/storage ready");

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let sw_interrupt =
        esp_hal::interrupt::software::SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    esp_rtos::start(timg0.timer0, sw_interrupt.software_interrupt0);

    let resources::Cpu0Resources {
        display: display_resources,
        camera: camera_resources,
    } = resources::Cpu0Resources {
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
    };
    let resources::Cpu1Resources {
        system_i2c: mut system_i2c_resources,
        audio: audio_resources,
        network: network_resources,
    } = resources::Cpu1Resources {
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
    };

    let mut delay = Delay::new();

    // Board bootstrap initially borrows the runtime I2C resources only long
    // enough to configure the PMIC and AW9523. The driver is then dropped so a
    // fresh camera SCCB owner can take I2C0 + GPIO12/GPIO11.
    let mut bootstrap_i2c = system_i2c::init(system_i2c_resources.reborrow());
    board::power::enable_lcd_backlight(&mut bootstrap_i2c);
    board::io_expander::reset_display_and_touch(&mut bootstrap_i2c, &mut delay);

    // CoreS3/CoreS3-Lite power the GC0308 and its onboard 20 MHz oscillator
    // from AXP2101 ALDO3. Camera bring-up remains optional: a camera fault must
    // not prevent the rest of the device from booting.
    let camera_powered_and_reset = power_and_reset_camera(&mut bootstrap_i2c, &mut delay);

    // M5Stack's CoreS3 camera fix explicitly releases the shared internal-I2C
    // owner before camera initialization. Do the same: destroy the bootstrap
    // driver, create a fresh 100 kHz hardware SCCB owner, program/read GC0308,
    // then destroy that owner before creating the persistent runtime bus.
    drop(bootstrap_i2c);
    let camera_ready = if camera_powered_and_reset {
        let mut camera_sccb = system_i2c::init_camera_sccb(system_i2c_resources.reborrow());
        let ready = initialize_camera_sensor(&mut camera_sccb, &mut delay);
        drop(camera_sccb);
        ready
    } else {
        false
    };

    // Camera SCCB is gone now. Construct the final 400 kHz system-I2C owner;
    // this exact driver is later moved to CPU1 and converted to async mode.
    let mut system_i2c = system_i2c::init(system_i2c_resources);

    let display = display::init(display_resources, &mut delay);
    let camera = camera::init(camera_resources);

    audio::init_es7210(&mut system_i2c).expect("Failed to initialize ES7210 microphone codec");
    audio::init_aw88298(&mut system_i2c, &mut delay)
        .expect("Failed to initialize AW88298 speaker amplifier");

    info!("==========================================");
    info!(">>> M5Stack CoreS3 Lite Booting Up! <<<");
    info!("==========================================");

    memory::report("before CPU1 startup");
    info!("Starting CPU1 acquisition executor");

    let audio::Endpoints {
        runtime: audio_runtime,
        input: audio_input,
        playback,
    } = audio::init_endpoints();
    let imu::Endpoints {
        runtime: imu_runtime,
        input: imu_input,
    } = imu::init_endpoints();
    let network::Endpoints {
        runtime: network_runtime,
        input: network_input,
    } = network::init_endpoints();
    let touch::Endpoints {
        runtime: touch_runtime,
        input: touch_input,
    } = touch::init_endpoints();
    let display::BrightnessEndpoints {
        runtime: display_runtime,
        control: brightness,
    } = display::init_brightness_endpoints();

    let cpu1_endpoints = cpu1::ServiceEndpoints {
        audio: audio_runtime,
        imu: imu_runtime,
        network: network_runtime,
        touch: touch_runtime,
        display: display_runtime,
    };

    let cpu1_stack = cpu1::init_stack();
    esp_rtos::start_second_core(
        peripherals.CPU_CTRL,
        sw_interrupt.software_interrupt1,
        cpu1_stack,
        move || {
            cpu1::run(
                system_i2c,
                audio_resources,
                network_resources,
                cpu1_endpoints,
            )
        },
    );

    Bootstrap {
        display,
        camera,
        camera_ready,
        inputs: AppInputs {
            touch: touch_input,
            imu: imu_input,
            audio: audio_input,
            network: network_input,
            log: log_input,
        },
        brightness,
        playback,
    }
}
