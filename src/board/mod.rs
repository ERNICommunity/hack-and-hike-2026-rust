//! The M5Stack CoreS3 Lite board: its physical facts and its bring-up.
//!
//! This module contains what belongs to the circuit board and not to one
//! chip:
//!
//! - the shared I2C bus (the two-wire bus to most chips on the board),
//! - the power rails of the AXP2101 power chip,
//! - the reset lines on the AW9523 IO expander (a chip that adds extra pins),
//! - PSRAM, the external RAM chip,
//! - the display size and orientation,
//! - the pins and DMA channels that each capability gets.
//!
//! The register setup of a chip stays in the capability that uses it: LCD
//! controller setup in `display`, codec setup in `audio`, sensor setup in
//! `imu` and `light`. Applications see only [`Board`] and [`psram`]. The
//! crate root re-exports both.
//!
//! [`Board::init`] starts the CoreS3 Lite hardware in the required order. It
//! starts the capability tasks on CPU1 and returns one handle for each
//! capability. The application keeps the handles it needs and drops the
//! rest. The CPU1 tasks keep running in both cases.
//!
//! Bring-up (the start-up of the hardware) stops at once when a required chip
//! is missing. The power chip, the IO expander and the audio codecs must
//! answer on I2C. If one does not, `init` panics with a message that names
//! the chip. The camera and the light and proximity sensor are optional.
//! When they do not answer, their fields are `None`. The IMU and the touch
//! controller are first contacted from CPU1. There, a failure does not stop
//! the board: the IMU logs it and sets itself up again, and touch skips the
//! failed read and polls again.
//!
//! The order of the steps matters:
//!
//! 1. Heaps and the logger, so all later steps can allocate and log.
//! 2. PSRAM and the log history, then the RTOS timer.
//! 3. I2C bus recovery. Then the power chip turns on the backlight rail, and
//!    the IO expander resets the display and the touch controller. These two
//!    chips do not work before this step.
//! 4. The camera: its power rails, its reset, and its setup over the shared
//!    I2C pins at a slower speed.
//! 5. The display over SPI.
//! 6. The audio codecs and a check for the light and proximity sensor. Then
//!    the system I2C bus moves to CPU1.
//! 7. CPU1 starts the IMU, touch, light, audio, radio and backlight tasks.

mod cpu1;
pub(crate) mod i2c;
pub(crate) mod io_expander;
pub(crate) mod power;
pub mod psram;
pub(crate) mod registers;

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
    logging::{self, LogHistory},
};

/// Width of the LCD and of the touch panel's coordinate space.
pub(crate) const DISPLAY_WIDTH: usize = 320;
/// Height of the LCD and of the touch panel's coordinate space.
pub(crate) const DISPLAY_HEIGHT: usize = 240;

/// Whether the panel is mounted upside down, compared with the native
/// orientation of its controllers. The LCD setup and the touch coordinates
/// both use this value. So a drawn point and a touched point use the same
/// coordinates.
pub(crate) const DISPLAY_ROTATED_180: bool = true;

/// Convert a point from the native coordinates of the touch controller into
/// display coordinates.
pub(crate) const fn logical_display_point(x: u16, y: u16) -> (u16, u16) {
    if DISPLAY_ROTATED_180 {
        hack_and_hike_core::touch::rotate_180(x, y, DISPLAY_WIDTH as u16, DISPLAY_HEIGHT as u16)
    } else {
        (x, y)
    }
}

/// Number of attempts for the first I2C transaction after a reset. The chip
/// may still be starting after the reset or after its power came on.
const FIRST_CONTACT_ATTEMPTS: u32 = 5;
/// Pause between two of those attempts, in milliseconds.
const FIRST_CONTACT_RETRY_MS: u32 = 10;

/// Run `attempt` until it succeeds, at most [`FIRST_CONTACT_ATTEMPTS`] times.
///
/// Logs each failure that is followed by another attempt. Returns the result
/// of the last attempt, so the caller handles the final error.
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

/// Size of a heap in the internal RAM that the second-stage bootloader used.
/// The bootloader has finished when the application runs, so this RAM is
/// free. The value is the esp-generate default for the ESP32-S3.
const RECLAIMED_HEAP_BYTES: usize = 73744;
/// Size of a second heap in internal RAM, taken from DRAM (the data RAM of
/// the application). PSRAM is a separate heap; see [`psram`].
const INTERNAL_HEAP_BYTES: usize = 72 * 1024;

/// One handle for each capability of the CoreS3 Lite.
///
/// Use a pattern to keep the handles your application needs. The `..` drops
/// the rest:
///
/// ```ignore
/// let Board { mut display, mut imu, .. } = Board::init();
/// ```
///
/// Each handle exists only once. So the code that owns a handle is the only
/// code that can use that part of the hardware. Dropping a handle does not
/// stop the hardware: the tasks on CPU1 keep running.
pub struct Board {
    /// The 320x240 pixel LCD screen.
    pub display: Display,
    /// The brightness of the screen's backlight.
    pub backlight: Backlight,
    /// The touch panel on top of the screen.
    pub touch: Touch,
    /// The IMU (inertial measurement unit): accelerometer, gyroscope and
    /// magnetometer, combined into roll, pitch and compass heading.
    pub imu: Imu,
    /// The two microphones, as blocks of 16 kHz stereo samples.
    pub microphone: Microphone,
    /// The loudspeaker. It plays 16 kHz stereo samples.
    pub speaker: Speaker,
    /// Messages to and from nearby boards over ESP-NOW, a direct radio
    /// protocol from Espressif, the maker of the chip.
    pub network: Network,
    /// The camera. `None` when the camera did not answer in `Board::init`.
    pub camera: Option<Camera>,
    /// The ambient light sensor. `None` when the sensor did not answer in
    /// `Board::init`.
    pub light: Option<Light>,
    /// The proximity sensor: how close something is to the front of the
    /// board. It is the same chip as `light`, so `light` and `proximity` are
    /// both `Some` or both `None`.
    pub proximity: Option<Proximity>,
    /// The newest lines written with the `log` macros.
    pub log: LogHistory,
}

impl Board {
    /// Start the whole board. Call this once, as the first thing in `main`.
    ///
    /// It takes about half a second. Most of that time is spent waiting for
    /// chips to finish their reset. At the end, it starts the capability
    /// tasks on CPU1, the second CPU core.
    ///
    /// # Panics
    ///
    /// - When it is called a second time.
    /// - When the power chip (AXP2101), the IO expander (AW9523) or the audio
    ///   codecs do not answer on the I2C bus. The panic message names the
    ///   chip. First, switch the board off and on again (unplug USB).
    pub fn init() -> Self {
        esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: RECLAIMED_HEAP_BYTES);
        esp_alloc::heap_allocator!(size: INTERNAL_HEAP_BYTES);

        logging::init(LevelFilter::Info);
        info!("M5Stack CoreS3 Lite booting");

        let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
        let peripherals = esp_hal::init(config);

        psram::enable(peripherals.PSRAM);
        let log = logging::enable_history();
        logging::report_memory("after PSRAM setup");

        let timg0 = TimerGroup::new(peripherals.TIMG0);
        esp_rtos::start(timg0.timer0, peripherals.FROM_CPU_INTR0);

        let delay = Delay::new();
        let mut i2c_resources = i2c::Resources {
            i2c0: peripherals.I2C0,
            sda: peripherals.GPIO12,
            scl: peripherals.GPIO11,
        };

        // A reset in the middle of a CPU1 read can leave a chip that holds
        // the bus. Free the bus before the first transaction.
        i2c::recover_bus(&mut i2c_resources, delay);

        // A short-lived I2C driver sets the power rails and the reset lines.
        // The driver is dropped at the end of this block. That frees the pins,
        // so the camera can create its own slower driver on them.
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

        // This is the final system I2C driver. It moves to CPU1 after the
        // codec setup and the sensor check.
        let mut system_i2c = i2c::init(i2c_resources);
        audio::init_codecs(&mut system_i2c, delay).expect("audio codecs did not answer");
        // The light and proximity sensor is optional, like the camera. One
        // chip serves both handles. Its task starts only when the chip
        // answered.
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
        // application, and the runtime for its CPU1 task. Both use the same
        // queues or signals.

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

        logging::report_memory("before CPU1 start");
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
