#[cfg(any(feature = "display", feature = "touch", feature = "imu"))]
use embassy_sync::{blocking_mutex::raw::NoopRawMutex, mutex::Mutex};
use esp_hal::{
    Blocking,
    i2c::master::{Config as I2cConfig, I2c},
    peripherals::{GPIO11, GPIO12, I2C0},
    time::Rate,
};
#[cfg(any(feature = "display", feature = "touch", feature = "imu"))]
use esp_hal::Async;
#[cfg(any(feature = "display", feature = "touch", feature = "imu"))]
use static_cell::StaticCell;

const SYSTEM_I2C_FREQUENCY_KHZ: u32 = 400;
#[cfg(feature = "camera")]
const CAMERA_SCCB_FREQUENCY_KHZ: u32 = 100;

/// Physical resources for the board's runtime system-I2C service.
///
/// CPU0 may temporarily reborrow these resources during bootstrap. The final
/// owning driver is then moved to CPU1 and converted to async for runtime
/// touch/IMU/display-control use.
pub(crate) struct Resources<'d> {
    pub(crate) i2c0: I2C0<'d>,
    pub(crate) sda: GPIO12<'d>,
    pub(crate) scl: GPIO11<'d>,
}

#[cfg(any(feature = "display", feature = "touch", feature = "camera"))]
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

/// CPU0 startup form that is ultimately moved to CPU1.
#[cfg(any(feature = "display", feature = "touch", feature = "imu"))]
pub(crate) type SystemI2cBlocking = I2c<'static, Blocking>;

/// CPU1 runtime form. ESP-HAL async drivers are core-affine because their
/// interrupt handler is installed on the core that calls `into_async()`.
#[cfg(any(feature = "display", feature = "touch", feature = "imu"))]
type SystemI2c = I2c<'static, Async>;

/// Runtime I2C is intentionally local to the CPU1 Embassy executor.
#[cfg(any(feature = "display", feature = "touch", feature = "imu"))]
type SystemI2cMutex = Mutex<NoopRawMutex, SystemI2c>;
#[cfg(any(feature = "display", feature = "touch", feature = "imu"))]
pub(crate) type SystemI2cBus = &'static SystemI2cMutex;

#[cfg(any(feature = "display", feature = "touch", feature = "imu"))]
static SYSTEM_I2C: StaticCell<SystemI2cMutex> = StaticCell::new();

fn init_with_frequency<'d>(resources: Resources<'d>, frequency_khz: u32) -> I2c<'d, Blocking> {
    let Resources { i2c0, sda, scl } = resources;

    I2c::new(
        i2c0,
        I2cConfig::default().with_frequency(Rate::from_khz(frequency_khz)),
    )
    .expect("Failed to configure system I2C")
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
#[cfg(feature = "camera")]
pub(crate) fn init_camera_sccb<'d>(resources: Resources<'d>) -> I2c<'d, Blocking> {
    init_with_frequency(resources, CAMERA_SCCB_FREQUENCY_KHZ)
}

/// Convert the already-configured system bus to async mode on CPU1 and publish
/// it to tasks running on that same executor.
///
/// This function must be called on CPU1. `I2c<Async>` is intentionally !Send:
/// ESP-HAL installs the driver's interrupt handler on the calling core.
#[cfg(any(feature = "display", feature = "touch", feature = "imu"))]
pub(crate) fn into_async(i2c: SystemI2cBlocking) -> SystemI2cBus {
    let i2c = i2c.into_async();
    SYSTEM_I2C.init(Mutex::new(i2c))
}
