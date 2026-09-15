//! The board's shared I2C bus.
//!
//! I2C is a two-wire bus: SDA carries the data and SCL the clock. One pair of
//! pins (SDA on GPIO12, SCL on GPIO11) connects the power chip, the IO
//! expander, the audio codecs, the IMU, the touch controller, the light and
//! proximity sensor and the camera sensor.
//!
//! During bring-up, CPU0 creates short-lived drivers on these pins. After
//! that, one async driver lives on CPU1. The capability tasks share it
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
/// Bus speed while the camera sensor is programmed. Its SCCB interface (the
/// camera's version of I2C) is slower than the other chips.
const CAMERA_SCCB_FREQUENCY_KHZ: u32 = 100;
/// Half a clock period while the bus is recovered by hand. 5 µs high and
/// 5 µs low give 100 kHz.
const RECOVERY_HALF_PERIOD_US: u32 = 5;

/// The I2C controller and its two pins.
///
/// During bring-up, CPU0 reborrows them for temporary drivers. The final
/// driver takes ownership of them and moves to CPU1.
pub(crate) struct Resources<'d> {
    /// The I2C controller.
    pub(crate) i2c0: I2C0<'d>,
    /// Data line.
    pub(crate) sda: GPIO12<'d>,
    /// Clock line.
    pub(crate) scl: GPIO11<'d>,
}

impl Resources<'static> {
    /// Borrow all three resources for a temporary driver, and keep the
    /// `'static` ownership here.
    ///
    /// When the temporary driver is dropped, it releases the controller and
    /// the pins. Then the next driver can use them safely.
    pub(crate) fn reborrow(&mut self) -> Resources<'_> {
        Resources {
            i2c0: self.i2c0.reborrow(),
            sda: self.sda.reborrow(),
            scl: self.scl.reborrow(),
        }
    }
}

/// Free the bus if a chip still holds it from before the last reset.
///
/// CPU1 reads the touch controller and the IMU all the time. So a reset (for
/// example by the flasher) often happens in the middle of a read. The chip
/// then keeps SDA low and waits for clock pulses that never come. Every
/// transaction after the reset fails, because the chip does not acknowledge.
///
/// This function drives SCL by hand, so the chip can finish its byte. Then a
/// STOP condition returns every chip to idle. Call this before the first I2C
/// driver is created.
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

    // At most nine clock pulses: the rest of one byte plus its acknowledge
    // bit. Stop early when the chip releases SDA.
    for _ in 0..9 {
        scl.set_low();
        delay.delay_micros(RECOVERY_HALF_PERIOD_US);
        scl.set_high();
        delay.delay_micros(RECOVERY_HALF_PERIOD_US);
        if sda.is_high() {
            break;
        }
    }
    // SCL is high here. SDA falling while SCL is high is a START condition,
    // and SDA rising while SCL is high is a STOP condition. START followed by
    // STOP ends any transfer that a chip still expects.
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

/// The system bus driver in blocking mode. CPU0 uses it during bring-up and
/// then moves it to CPU1.
pub(crate) type SystemI2cBlocking = I2c<'static, Blocking>;

/// The system bus driver in async mode, on CPU1. An esp-hal async driver
/// must stay on one core: `into_async()` installs its interrupt handler on
/// the core that calls it.
type SystemI2c = I2c<'static, Async>;

/// The mutex around the async driver. Only tasks on the CPU1 executor use
/// it, so a mutex without protection against interrupts or the other core is
/// enough. (An executor is the part of the async runtime that runs tasks.)
type SystemI2cMutex = Mutex<NoopRawMutex, SystemI2c>;
/// How CPU1 tasks share the bus: lock it for one transaction at a time.
pub(crate) type SystemI2cBus = &'static SystemI2cMutex;

/// Storage for the shared bus mutex. A `StaticCell` gives out one `&'static`
/// reference at run time. Tasks that run forever can share that reference.
static SYSTEM_I2C: StaticCell<SystemI2cMutex> = StaticCell::new();

/// Create a blocking driver on `resources` at `frequency_khz`.
///
/// # Panics
///
/// When esp-hal rejects the configuration. This does not happen with the
/// fixed frequencies in this module.
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

/// Create a blocking driver for the system bus at 400 kHz.
///
/// The lifetime `'d` can be short. So bring-up can create and drop temporary
/// drivers before it creates the final `'static` driver for CPU1.
pub(crate) fn init<'d>(resources: Resources<'d>) -> I2c<'d, Blocking> {
    init_with_frequency(resources, SYSTEM_I2C_FREQUENCY_KHZ)
}

/// Create a blocking driver at 100 kHz for programming the GC0308 camera
/// sensor during bring-up.
///
/// M5Stack's CoreS3 camera code does the same: it releases the shared I2C
/// driver, and the camera creates its own SCCB driver on GPIO12 and GPIO11.
/// This driver is separate from the system bus driver that CPU1 uses later.
pub(crate) fn init_camera_sccb<'d>(resources: Resources<'d>) -> I2c<'d, Blocking> {
    init_with_frequency(resources, CAMERA_SCCB_FREQUENCY_KHZ)
}

/// Convert the system bus driver to async mode and store it for the tasks
/// on the CPU1 executor.
///
/// Call this on CPU1. esp-hal installs the interrupt handler of the driver on
/// the calling core. For this reason `I2c<Async>` is not `Send`: it cannot
/// move to another core.
///
/// # Panics
///
/// When it is called a second time.
pub(crate) fn into_async(i2c: SystemI2cBlocking) -> SystemI2cBus {
    let i2c = i2c.into_async();
    SYSTEM_I2C.init(Mutex::new(i2c))
}
