//! Board bring-up.
//!
//! [`Board::init`] powers the CoreS3 Lite hardware in the required order,
//! starts the CPU1 capability runtimes and returns one handle per capability.
//! The application takes ownership of the handles it needs and drops the rest;
//! the CPU1 runtimes keep running either way.
//!
//! Bring-up is fail-fast: a peripheral that does not answer on I2C panics with
//! a message naming it, because the board is unusable without it. The camera
//! is the one exception and simply comes back as `None`.

mod cpu1;

use esp_hal::{clock::CpuClock, delay::Delay, timer::timg::TimerGroup};
use log::{LevelFilter, info, warn};

use crate::{
    capabilities::{
        audio::{self, Microphone, Speaker},
        backlight::{self, Backlight},
        camera::{self, Camera},
        display::{self, Display},
        imu::{self, Imu},
        network::{self, Network},
        touch::{self, Touch},
    },
    platform::{i2c, io_expander, power},
    support::{
        logging::{self, LogHistory},
        memory,
    },
};

/// The first I2C transaction after a reset gets a few attempts: a chip may
/// still be settling from the reset or the power-up of its rail.
const FIRST_CONTACT_ATTEMPTS: u32 = 5;
const FIRST_CONTACT_RETRY_MS: u32 = 10;

fn retry<T, E: core::fmt::Debug>(
    delay: Delay,
    mut attempt: impl FnMut() -> Result<T, E>,
) -> Result<T, E> {
    let mut result = attempt();
    for _ in 1..FIRST_CONTACT_ATTEMPTS {
        let Err(error) = &result else { break };
        warn!("I2C chip did not answer ({error:?}); retrying");
        delay.delay_millis(FIRST_CONTACT_RETRY_MS);
        result = attempt();
    }
    result
}

/// Internal RAM that the second-stage bootloader no longer needs once the
/// application runs (the esp-generate default for the ESP32-S3).
const RECLAIMED_HEAP_BYTES: usize = 73744;
/// Additional internal heap carved from DRAM. PSRAM is a separate heap; see
/// [`crate::support::memory`].
const INTERNAL_HEAP_BYTES: usize = 72 * 1024;

/// One handle per capability of the CoreS3 Lite.
///
/// Destructure it and keep what your application needs:
///
/// ```ignore
/// let Board { mut display, mut imu, .. } = Board::init();
/// ```
pub struct Board {
    pub display: Display,
    pub backlight: Backlight,
    pub touch: Touch,
    pub imu: Imu,
    pub microphone: Microphone,
    pub speaker: Speaker,
    pub network: Network,
    /// `None` when the camera module did not answer during bring-up.
    pub camera: Option<Camera>,
    /// History of everything written through the `log` macros.
    pub log: LogHistory,
}

impl Board {
    /// Bring up the whole board. Call this once, first thing in `main`.
    pub fn init() -> Self {
        esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: RECLAIMED_HEAP_BYTES);
        esp_alloc::heap_allocator!(size: INTERNAL_HEAP_BYTES);

        logging::init(LevelFilter::Info);
        info!("M5Stack CoreS3 Lite booting");

        let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
        let peripherals = esp_hal::init(config);

        memory::enable_psram(peripherals.PSRAM);
        let log = logging::enable_history();
        memory::report("after PSRAM setup");

        let timg0 = TimerGroup::new(peripherals.TIMG0);
        esp_rtos::start(timg0.timer0, peripherals.FROM_CPU_INTR0);

        let delay = Delay::new();
        let mut i2c_resources = i2c::Resources {
            i2c0: peripherals.I2C0,
            sda: peripherals.GPIO12,
            scl: peripherals.GPIO11,
        };

        // Power rails and reset lines are driven over a short-lived I2C owner;
        // dropping it frees the pins for the camera's slower bus.
        i2c::recover_bus(&mut i2c_resources, delay);
        {
            let mut i2c = i2c::init(i2c_resources.reborrow());
            retry(delay, || power::enable_lcd_backlight(&mut i2c))
                .expect("AXP2101 power chip did not answer");
            io_expander::reset_display_and_touch(&mut i2c, delay)
                .expect("AW9523 IO expander did not answer");
        }

        let camera = camera::bring_up(
            &mut i2c_resources,
            delay,
            camera::Resources {
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
        );

        let display = display::init(
            display::Resources {
                spi2: peripherals.SPI2,
                dma: peripherals.DMA_CH1,
                sck: peripherals.GPIO36,
                mosi: peripherals.GPIO37,
                dc: peripherals.GPIO35,
                cs: peripherals.GPIO3,
            },
            delay,
        );

        // The final system I2C driver moves to CPU1 once the codecs are set up.
        let mut system_i2c = i2c::init(i2c_resources);
        audio::init_codecs(&mut system_i2c, delay).expect("audio codecs did not answer");

        let audio::Endpoints {
            microphone,
            speaker,
            runtime: audio_runtime,
        } = audio::endpoints();
        let backlight::Endpoints {
            handle: backlight,
            runtime: backlight_runtime,
        } = backlight::endpoints();
        let imu::Endpoints {
            handle: imu,
            runtime: imu_runtime,
        } = imu::endpoints();
        let network::Endpoints {
            handle: network,
            runtime: network_runtime,
        } = network::endpoints();
        let touch::Endpoints {
            handle: touch,
            runtime: touch_runtime,
        } = touch::endpoints();

        memory::report("before CPU1 start");
        cpu1::start(
            peripherals.CPU_CTRL,
            peripherals.FROM_CPU_INTR1,
            cpu1::Cpu1 {
                system_i2c,
                audio_resources: audio::Resources {
                    i2s0: peripherals.I2S0,
                    dma: peripherals.DMA_CH0,
                    mclk: peripherals.GPIO0,
                    bclk: peripherals.GPIO34,
                    word_select: peripherals.GPIO33,
                    data_in: peripherals.GPIO14,
                    data_out: peripherals.GPIO13,
                },
                network_resources: network::Resources {
                    wifi: peripherals.WIFI,
                },
                audio: audio_runtime,
                backlight: backlight_runtime,
                imu: imu_runtime,
                network: network_runtime,
                touch: touch_runtime,
            },
        );

        Board {
            display,
            backlight,
            touch,
            imu,
            microphone,
            speaker,
            network,
            camera,
            log,
        }
    }
}
