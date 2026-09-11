//! CoreS3 Lite bootstrap and concrete firmware composition.
//!
//! Enabled capabilities own their hardware. Shared-I2C ordering remains explicit:
//! optional board setup, optional camera SCCB, then the final runtime bus used by
//! enabled I2C-backed capabilities.

use ::log::info;
#[cfg(feature = "camera")]
use ::log::warn;
use esp_hal::{clock::CpuClock, delay::Delay, timer::timg::TimerGroup};

#[cfg(any(feature = "display", feature = "touch", feature = "camera"))]
use crate::platform::board;
#[cfg(any(
    feature = "display",
    feature = "touch",
    feature = "imu",
    feature = "mic",
    feature = "speaker",
    feature = "camera",
))]
use crate::platform::i2c as system_i2c;
#[cfg(any(feature = "mic", feature = "speaker"))]
use crate::capabilities::audio;
#[cfg(feature = "camera")]
use crate::capabilities::camera;
#[cfg(feature = "display")]
use crate::capabilities::display;
#[cfg(feature = "imu")]
use crate::capabilities::imu;
#[cfg(feature = "mic")]
use crate::capabilities::mic;
#[cfg(feature = "network")]
use crate::capabilities::network;
#[cfg(feature = "touch")]
use crate::capabilities::touch;
use crate::{
    firmware::cpu1,
    support::{logging as logger, memory},
};

pub(crate) struct AppInputs {
    #[cfg(feature = "touch")]
    pub(crate) touch: touch::Input,
    #[cfg(feature = "imu")]
    pub(crate) imu: imu::Imu,
    #[cfg(feature = "mic")]
    pub(crate) microphone: mic::Microphone,
    #[cfg(feature = "network")]
    pub(crate) network: network::Input,
    #[cfg(feature = "log-view")]
    pub(crate) log: logger::Input,
}

pub(crate) struct Bootstrap {
    #[cfg(feature = "display")]
    pub(crate) display: display::Display,
    #[cfg(feature = "camera")]
    pub(crate) camera: camera::Camera,
    #[cfg(feature = "camera")]
    pub(crate) camera_ready: bool,
    pub(crate) inputs: AppInputs,
    #[cfg(feature = "display")]
    pub(crate) brightness: display::BrightnessControl,
    #[cfg(feature = "speaker-synth")]
    pub(crate) playback: audio::PlaybackControl,
}

#[cfg(feature = "camera")]
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

#[cfg(feature = "camera")]
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
                pid, camera::EXPECTED_SENSOR_PID
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
    #[cfg(feature = "log-view")]
    let log_input = logger::enable_psram_history();
    memory::report("PSRAM/storage ready");

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let sw_interrupt =
        esp_hal::interrupt::software::SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    esp_rtos::start(timg0.timer0, sw_interrupt.software_interrupt0);

    #[cfg(feature = "display")]
    let display_resources = display::Resources {
        spi2: peripherals.SPI2,
        dma: peripherals.DMA_CH1,
        sck: peripherals.GPIO36,
        mosi: peripherals.GPIO37,
        dc: peripherals.GPIO35,
        cs: peripherals.GPIO3,
    };
    #[cfg(feature = "camera")]
    let camera_resources = camera::Resources {
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
    };

    #[cfg(any(
        feature = "display",
        feature = "touch",
        feature = "imu",
        feature = "mic",
        feature = "speaker",
        feature = "camera",
    ))]
    let mut system_i2c_resources = system_i2c::Resources {
        i2c0: peripherals.I2C0,
        sda: peripherals.GPIO12,
        scl: peripherals.GPIO11,
    };

    #[cfg(any(feature = "mic", feature = "speaker"))]
    let audio_resources = audio::Resources {
        i2s0: peripherals.I2S0,
        dma: peripherals.DMA_CH0,
        mclk: peripherals.GPIO0,
        bclk: peripherals.GPIO34,
        word_select: peripherals.GPIO33,
        #[cfg(feature = "mic")]
        data_in: peripherals.GPIO14,
        #[cfg(feature = "speaker")]
        data_out: peripherals.GPIO13,
    };
    #[cfg(feature = "network")]
    let network_resources = network::Resources {
        wifi: peripherals.WIFI,
    };

    let mut delay = Delay::new();

    #[cfg(any(feature = "display", feature = "touch", feature = "camera"))]
    let mut bootstrap_i2c = system_i2c::init(system_i2c_resources.reborrow());
    #[cfg(feature = "display")]
    board::power::enable_lcd_backlight(&mut bootstrap_i2c);
    #[cfg(all(feature = "display", feature = "touch"))]
    board::io_expander::reset_display_and_touch(&mut bootstrap_i2c, &mut delay);
    #[cfg(all(feature = "display", not(feature = "touch")))]
    board::io_expander::reset_display(&mut bootstrap_i2c, &mut delay);
    #[cfg(all(feature = "touch", not(feature = "display")))]
    board::io_expander::reset_touch(&mut bootstrap_i2c, &mut delay);
    #[cfg(feature = "camera")]
    let camera_powered_and_reset = power_and_reset_camera(&mut bootstrap_i2c, &mut delay);
    #[cfg(any(feature = "display", feature = "touch", feature = "camera"))]
    drop(bootstrap_i2c);

    #[cfg(feature = "camera")]
    let camera_ready = if camera_powered_and_reset {
        let mut camera_sccb = system_i2c::init_camera_sccb(system_i2c_resources.reborrow());
        let ready = initialize_camera_sensor(&mut camera_sccb, &mut delay);
        drop(camera_sccb);
        ready
    } else {
        false
    };

    #[cfg(any(
        feature = "display",
        feature = "touch",
        feature = "imu",
        feature = "mic",
        feature = "speaker",
    ))]
    let mut system_i2c = system_i2c::init(system_i2c_resources);

    #[cfg(feature = "display")]
    let display = display::init(display_resources, &mut delay);
    #[cfg(feature = "camera")]
    let camera = camera::init(camera_resources);

    #[cfg(feature = "mic")]
    audio::init_es7210(&mut system_i2c).expect("Failed to initialize ES7210 microphone codec");
    #[cfg(feature = "speaker")]
    audio::init_aw88298(&mut system_i2c, &mut delay)
        .expect("Failed to initialize AW88298 speaker amplifier");

    info!("==========================================");
    info!(">>> M5Stack CoreS3 Lite Booting Up! <<<");
    info!("==========================================");

    #[cfg(any(feature = "mic", feature = "speaker"))]
    let audio_endpoints = audio::init_endpoints();
    #[cfg(any(feature = "mic", feature = "speaker"))]
    let audio_runtime = audio_endpoints.runtime;
    #[cfg(feature = "mic")]
    let microphone = mic::from_reader(audio_endpoints.mic);
    #[cfg(feature = "speaker-synth")]
    let playback = audio_endpoints.playback;

    #[cfg(feature = "imu")]
    let imu::Endpoints {
        runtime: imu_runtime,
        input: imu_input,
    } = imu::init_endpoints();
    #[cfg(feature = "network")]
    let network::Endpoints {
        runtime: network_runtime,
        input: network_input,
    } = network::init_endpoints();
    #[cfg(feature = "touch")]
    let touch::Endpoints {
        runtime: touch_runtime,
        input: touch_input,
    } = touch::init_endpoints();
    #[cfg(feature = "display")]
    let display::BrightnessEndpoints {
        runtime: display_runtime,
        control: brightness,
    } = display::init_brightness_endpoints();

    #[cfg(any(
        feature = "display",
        feature = "touch",
        feature = "imu",
        feature = "mic",
        feature = "speaker",
        feature = "network",
    ))]
    {
        memory::report("before CPU1 startup");
        info!("Starting CPU1 acquisition executor");

        let cpu1_endpoints = cpu1::CapabilityEndpoints {
            #[cfg(any(feature = "mic", feature = "speaker"))]
            audio: audio_runtime,
            #[cfg(feature = "imu")]
            imu: imu_runtime,
            #[cfg(feature = "network")]
            network: network_runtime,
            #[cfg(feature = "touch")]
            touch: touch_runtime,
            #[cfg(feature = "display")]
            display: display_runtime,
        };

        let cpu1_stack = cpu1::init_stack();
        esp_rtos::start_second_core(
            peripherals.CPU_CTRL,
            sw_interrupt.software_interrupt1,
            cpu1_stack,
            move || {
                cpu1::run(
                    #[cfg(any(feature = "display", feature = "imu", feature = "touch"))]
                    system_i2c,
                    #[cfg(any(feature = "mic", feature = "speaker"))]
                    audio_resources,
                    #[cfg(feature = "network")]
                    network_resources,
                    cpu1_endpoints,
                )
            },
        );
    }

    #[cfg(all(
        any(feature = "mic", feature = "speaker"),
        not(any(feature = "display", feature = "imu", feature = "touch")),
    ))]
    drop(system_i2c);

    Bootstrap {
        #[cfg(feature = "display")]
        display,
        #[cfg(feature = "camera")]
        camera,
        #[cfg(feature = "camera")]
        camera_ready,
        inputs: AppInputs {
            #[cfg(feature = "touch")]
            touch: touch_input,
            #[cfg(feature = "imu")]
            imu: imu_input,
            #[cfg(feature = "mic")]
            microphone,
            #[cfg(feature = "network")]
            network: network_input,
            #[cfg(feature = "log-view")]
            log: log_input,
        },
        #[cfg(feature = "display")]
        brightness,
        #[cfg(feature = "speaker-synth")]
        playback,
    }
}
