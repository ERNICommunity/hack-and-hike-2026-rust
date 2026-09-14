//! The board's shared I2C bus.
//!
//! One pair of pins (SDA on GPIO12, SCL on GPIO11) connects the power chip,
//! the IO expander, the audio codecs, the IMU, the touch controller and the
//! camera sensor. During bring-up CPU0 creates short-lived drivers on it;
//! afterwards one async driver lives on CPU1, shared by the capability tasks
//! through a mutex.

use embassy_sync::{blocking_mutex::raw::NoopRawMutex, mutex::Mutex};
use esp_hal::{
    Async, Blocking,
    delay::Delay,
    gpio::{DriveMode, Flex, OutputConfig, Pull},
    i2c::master::{Config as I2cConfig, I2c},
    peripherals::{GPIO11, GPIO12, I2C0},
    time::Rate,
};
use log::warn;
use static_cell::StaticCell;

/// Bus speed for every chip except the camera sensor.
const SYSTEM_I2C_FREQUENCY_KHZ: u32 = 400;
/// Bus speed while the camera sensor is programmed (its SCCB interface is
/// slower than the other chips).
const CAMERA_SCCB_FREQUENCY_KHZ: u32 = 100;
/// Half a clock period while recovering the bus by hand: 100 kHz.
const RECOVERY_HALF_PERIOD_US: u32 = 5;

/// The I2C controller and its two pins.
///
/// CPU0 reborrows these during bring-up for temporary drivers; the final
/// driver takes them for good and moves to CPU1.
pub(crate) struct Resources<'d> {
    /// The I2C controller.
    pub(crate) i2c0: I2C0<'d>,
    /// Data line.
    pub(crate) sda: GPIO12<'d>,
    /// Clock line.
    pub(crate) scl: GPIO11<'d>,
}

impl Resources<'static> {
    /// Borrow all three singleton resources without giving up their final
    /// `'static` ownership. Dropping a temporary driver releases the I2C/GPIO
    /// peripheral connections so another startup owner can use them safely.
    pub(crate) fn reborrow(&mut self) -> Resources<'_> {
        Resources {
            i2c0: self.i2c0.reborrow(),
            sda: self.sda.reborrow(),
            scl: self.scl.reborrow(),
        }
    }
}

/// Free the bus if a chip is still holding it from before the last reset.
///
/// CPU1 reads the touch controller and the IMU all the time, so a reset (for
/// example by the flasher) often lands in the middle of a read. The chip then
/// keeps SDA low, waiting for clocks that never come, and every transaction
/// after the reset fails with a missing acknowledge. Clocking SCL by hand
/// lets the chip finish its byte, and a STOP condition returns it to idle.
/// Call this before the first I2C driver is created.
pub(crate) fn recover_bus(resources: &mut Resources<'static>, delay: Delay) {
    let open_drain = OutputConfig::default()
        .with_drive_mode(DriveMode::OpenDrain)
        .with_pull(Pull::Up);
    let mut scl = Flex::new(resources.scl.reborrow());
    let mut sda = Flex::new(resources.sda.reborrow());
    for pin in [&mut scl, &mut sda] {
        pin.set_high();
        pin.apply_output_config(&open_drain);
        pin.set_input_enable(true);
        pin.set_output_enable(true);
    }
    delay.delay_micros(RECOVERY_HALF_PERIOD_US);
    if sda.is_high() {
        return;
    }

    // At most nine clocks: the rest of one byte plus its acknowledge bit.
    for _ in 0..9 {
        scl.set_low();
        delay.delay_micros(RECOVERY_HALF_PERIOD_US);
        scl.set_high();
        delay.delay_micros(RECOVERY_HALF_PERIOD_US);
        if sda.is_high() {
            break;
        }
    }
    // STOP: SDA rises while SCL is high.
    sda.set_low();
    delay.delay_micros(RECOVERY_HALF_PERIOD_US);
    scl.set_high();
    delay.delay_micros(RECOVERY_HALF_PERIOD_US);
    sda.set_high();
    delay.delay_micros(RECOVERY_HALF_PERIOD_US);

    if sda.is_high() {
        warn!("I2C bus was held by a chip after the reset; released it");
    } else {
        warn!("I2C bus is still held low after recovery; power-cycle the board");
    }
}

/// CPU0 startup form that is ultimately moved to CPU1.
pub(crate) type SystemI2cBlocking = I2c<'static, Blocking>;

/// CPU1 runtime form. ESP-HAL async drivers are core-affine because their
/// interrupt handler is installed on the core that calls `into_async()`.
type SystemI2c = I2c<'static, Async>;

/// The runtime bus is only used by tasks on the CPU1 executor, so a mutex
/// without interrupt or cross-core protection is enough.
type SystemI2cMutex = Mutex<NoopRawMutex, SystemI2c>;
/// How CPU1 tasks share the bus: lock it for one transaction at a time.
pub(crate) type SystemI2cBus = &'static SystemI2cMutex;

static SYSTEM_I2C: StaticCell<SystemI2cMutex> = StaticCell::new();

/// A blocking driver on `resources` at `frequency_khz`.
fn init_with_frequency<'d>(resources: Resources<'d>, frequency_khz: u32) -> I2c<'d, Blocking> {
    let Resources { i2c0, sda, scl } = resources;

    I2c::new(
        i2c0,
        I2cConfig::default().with_frequency(Rate::from_khz(frequency_khz)),
    )
    .expect("the I2C configuration is valid")
    .with_sda(sda)
    .with_scl(scl)
}

/// Configure the CoreS3-Lite internal system bus at 400 kHz in blocking mode.
///
/// This is generic over the resource lifetime so bootstrap can construct and
/// drop short-lived hardware-I2C owners before the final `'static` driver is
/// moved to CPU1.
pub(crate) fn init<'d>(resources: Resources<'d>) -> I2c<'d, Blocking> {
    init_with_frequency(resources, SYSTEM_I2C_FREQUENCY_KHZ)
}

/// Create the startup-only GC0308 control bus at 100 kHz.
///
/// M5Stack's CoreS3 camera code releases its shared internal I2C owner and lets
/// the camera create a fresh SCCB/I2C owner on GPIO12/GPIO11. Recreating the
/// ESP32-S3 hardware driver here mirrors that ownership boundary while keeping
/// the persistent runtime bus completely separate.
pub(crate) fn init_camera_sccb<'d>(resources: Resources<'d>) -> I2c<'d, Blocking> {
    init_with_frequency(resources, CAMERA_SCCB_FREQUENCY_KHZ)
}

/// Convert the already-configured system bus to async mode on CPU1 and publish
/// it to tasks running on that same executor.
///
/// This function must be called on CPU1. `I2c<Async>` is intentionally !Send:
/// ESP-HAL installs the driver's interrupt handler on the calling core.
pub(crate) fn into_async(i2c: SystemI2cBlocking) -> SystemI2cBus {
    let i2c = i2c.into_async();
    SYSTEM_I2C.init(Mutex::new(i2c))
}
