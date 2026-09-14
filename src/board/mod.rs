//! Board bring-up.
//!
//! [`Board::init`] powers the CoreS3 Lite hardware in the required order,
//! starts the CPU1 capability runtimes and returns one handle per capability.
//! The application takes ownership of the handles it needs and drops the
//! rest; the CPU1 runtimes keep running either way.
//!
//! Bring-up is fail-fast for the chips the board cannot work without: the
//! power chip, the IO expander and the audio codecs must answer on I2C, or
//! `init` panics with a message naming the chip. The camera and the light
//! and proximity sensor are optional and come back as `None`. The IMU and
//! the touch controller are first contacted from CPU1, where a failure is
//! logged and retried rather than fatal.
//!
//! The order of the steps matters:
//!
//! 1. Heaps and the logger, so everything after can allocate and log.
//! 2. PSRAM and the log history, the RTOS timer.
//! 3. I2C bus recovery, then the power chip and the IO expander: the display,
//!    touch controller and camera are unpowered or held in reset until then.
//! 4. The camera, which needs the shared I2C pins at a slower speed.
//! 5. The display over SPI.
//! 6. The audio codecs and a look for the light and proximity sensor, then
//!    the whole system I2C bus moves to CPU1.
//! 7. CPU1 starts the IMU, touch, light, audio, radio and backlight tasks.

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
        light::{self, Light},
        network::{self, Network},
        proximity::{self, Proximity},
        touch::{self, Touch},
    },
    platform::{i2c, io_expander, power},
    support::{
        logging::{self, LogHistory},
        memory,
    },
};

/// Attempts for the first I2C transaction after a reset: a chip may still be
/// settling from the reset or from the power-up of its rail.
const FIRST_CONTACT_ATTEMPTS: u32 = 5;
/// Pause between two of those attempts.
const FIRST_CONTACT_RETRY_MS: u32 = 10;

/// Run `attempt` until it succeeds, at most [`FIRST_CONTACT_ATTEMPTS`] times,
/// logging every failure. Returns the last result.
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
/// Destructure it and keep what your application needs; the `..` drops the
/// rest:
///
/// ```ignore
/// let Board { mut display, mut imu, .. } = Board::init();
/// ```
///
/// Each handle exists exactly once, so whoever owns it is the only code that
/// can use that piece of hardware.
pub struct Board {
    /// The 320x240 LCD.
    pub display: Display,
    /// The LCD backlight brightness.
    pub backlight: Backlight,
    /// The touch panel on top of the LCD.
    pub touch: Touch,
    /// Accelerometer, gyroscope and magnetometer, fused into an attitude.
    pub imu: Imu,
    /// The two microphones, as 16 kHz stereo blocks.
    pub microphone: Microphone,
    /// The loudspeaker, fed with 16 kHz stereo samples.
    pub speaker: Speaker,
    /// ESP-NOW messaging with nearby boards.
    pub network: Network,
    /// The camera; `None` when it did not answer during bring-up.
    pub camera: Option<Camera>,
    /// Ambient light; `None` when the sensor did not answer during bring-up.
    pub light: Option<Light>,
    /// How close something is to the front; `None` when the sensor did not
    /// answer during bring-up. The same chip as `light`.
    pub proximity: Option<Proximity>,
    /// History of everything written through the `log` macros.
    pub log: LogHistory,
}

impl Board {
    /// Bring up the whole board. Call this once, first thing in `main`.
    ///
    /// Takes about half a second, most of it waiting for chips to come out of
    /// reset.
    ///
    /// # Panics
    ///
    /// When called twice, or when the power chip, the IO expander or the
    /// audio codecs do not answer. The message names the chip; a
    /// power-cycle (unplug USB) is the first thing to try.
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

        // A reset in the middle of a CPU1 read can leave a chip holding the
        // bus; free it before the first transaction.
        i2c::recover_bus(&mut i2c_resources, delay);

        // Power rails and reset lines are driven over a short-lived I2C owner;
        // dropping it frees the pins for the camera's slower bus.
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
        // The light and proximity sensor is optional, like the camera: one
        // chip serves both handles, and its task is only spawned when the
        // chip answered.
        let (light, proximity, light_runtimes) = if light::probe(&mut system_i2c) {
            let light::Endpoints {
                handle: light,
                runtime: light_runtime,
            } = light::endpoints();
            let proximity::Endpoints {
                handle: proximity,
                runtime: proximity_runtime,
            } = proximity::endpoints();
            (
                Some(light),
                Some(proximity),
                Some((light_runtime, proximity_runtime)),
            )
        } else {
            (None, None, None)
        };

        // Every CPU1 capability comes as a pair: the handle for the
        // application and the runtime side for its CPU1 task, sharing one set
        // of queues.

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
                light: light_runtimes,
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
            light,
            proximity,
            log,
        }
    }
}
