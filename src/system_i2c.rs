use embassy_sync::{
    blocking_mutex::raw::NoopRawMutex,
    mutex::Mutex,
};
use esp_hal::{
    Async, Blocking,
    i2c::master::{Config as I2cConfig, I2c},
    peripherals::{GPIO11, GPIO12, I2C0},
    time::Rate,
};
use static_cell::StaticCell;

/// CPU0 startup form. Blocking drivers are Send, so this can be moved to CPU1
/// after one-time board initialization is complete.
pub type SystemI2cBlocking = I2c<'static, Blocking>;

/// CPU1 runtime form. ESP-HAL async drivers are core-affine because their
/// interrupt handler is installed on the core that calls `into_async()`.
pub type SystemI2c = I2c<'static, Async>;

/// Runtime I2C is intentionally local to the CPU1 Embassy executor.
///
/// `NoopRawMutex` is the correct Embassy raw mutex when all users are tasks on
/// one executor. The async mutex is still required because a transaction holds
/// exclusive ownership of the physical bus across `.await`.
pub type SystemI2cMutex = Mutex<NoopRawMutex, SystemI2c>;
pub type SystemI2cBus = &'static SystemI2cMutex;

static SYSTEM_I2C: StaticCell<SystemI2cMutex> = StaticCell::new();

/// Configure the CoreS3-Lite internal system bus at 400 kHz in blocking mode.
///
/// CPU0 uses this directly for one-time PMIC/AW9523/display/ES7210 setup. The
/// returned driver must then be moved to CPU1 and passed to [`into_async`].
pub fn init(
    i2c0: I2C0<'static>,
    gpio12: GPIO12<'static>,
    gpio11: GPIO11<'static>,
) -> SystemI2cBlocking {
    I2c::new(
        i2c0,
        I2cConfig::default().with_frequency(Rate::from_khz(400)),
    )
    .expect("Failed to configure system I2C")
    .with_sda(gpio12)
    .with_scl(gpio11)
}

/// Convert the already-configured system bus to async mode on CPU1 and publish
/// it to tasks running on that same executor.
///
/// This function must be called on CPU1. `I2c<Async>` is intentionally !Send:
/// ESP-HAL installs the driver's interrupt handler on the calling core.
pub fn into_async(i2c: SystemI2cBlocking) -> SystemI2cBus {
    let i2c = i2c.into_async();
    SYSTEM_I2C.init(Mutex::new(i2c))
}
