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

/// Physical resources for the board's runtime system-I2C service.
///
/// CPU0 may temporarily reborrow these resources during bootstrap. The final
/// owning driver is then moved to CPU1 and converted to async for runtime
/// touch/IMU/display-control use.
pub struct Resources<'d> {
    pub i2c0: I2C0<'d>,
    pub sda: GPIO12<'d>,
    pub scl: GPIO11<'d>,
}

impl Resources<'static> {
    /// Borrow all three singleton resources without giving up their final
    /// `'static` ownership. Dropping the temporary driver releases the GPIO
    /// peripheral connections so another startup owner can use them safely.
    pub fn reborrow(&mut self) -> Resources<'_> {
        Resources {
            i2c0: self.i2c0.reborrow(),
            sda: self.sda.reborrow(),
            scl: self.scl.reborrow(),
        }
    }
}

/// CPU0 startup form that is ultimately moved to CPU1.
pub type SystemI2cBlocking = I2c<'static, Blocking>;

/// CPU1 runtime form. ESP-HAL async drivers are core-affine because their
/// interrupt handler is installed on the core that calls `into_async()`.
pub type SystemI2c = I2c<'static, Async>;

/// Runtime I2C is intentionally local to the CPU1 Embassy executor.
pub type SystemI2cMutex = Mutex<NoopRawMutex, SystemI2c>;
pub type SystemI2cBus = &'static SystemI2cMutex;

static SYSTEM_I2C: StaticCell<SystemI2cMutex> = StaticCell::new();

/// Configure the CoreS3-Lite internal system bus at 400 kHz in blocking mode.
///
/// This is generic over the resource lifetime so bootstrap can construct and
/// drop a short-lived hardware-I2C driver before camera SCCB temporarily owns
/// GPIO12/GPIO11. Calling it later with `Resources<'static>` creates the driver
/// that is moved to CPU1.
pub fn init<'d>(resources: Resources<'d>) -> I2c<'d, Blocking> {
    let Resources { i2c0, sda, scl } = resources;

    I2c::new(
        i2c0,
        I2cConfig::default().with_frequency(Rate::from_khz(400)),
    )
    .expect("Failed to configure system I2C")
    .with_sda(sda)
    .with_scl(scl)
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
