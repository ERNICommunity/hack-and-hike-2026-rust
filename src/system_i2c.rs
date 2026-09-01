use core::cell::RefCell;

use embedded_hal_bus::i2c::RefCellDevice;
use esp_hal::{
    Blocking,
    i2c::master::{Config as I2cConfig, I2c},
    peripherals::{GPIO11, GPIO12, I2C0},
    time::Rate,
};
use static_cell::StaticCell;

pub type SystemI2c = I2c<'static, Blocking>;
pub type SystemI2cBus = &'static RefCell<SystemI2c>;
pub type SystemI2cDevice = RefCellDevice<'static, SystemI2c>;

static SYSTEM_I2C: StaticCell<RefCell<SystemI2c>> = StaticCell::new();

/// Initializes the CoreS3-Lite internal 400 kHz I2C bus once and returns the
/// shared bus cell used by all internal I2C devices.
pub fn init(
    i2c0: I2C0<'static>,
    gpio12: GPIO12<'static>,
    gpio11: GPIO11<'static>,
) -> SystemI2cBus {
    let i2c = I2c::new(
        i2c0,
        I2cConfig::default().with_frequency(Rate::from_khz(400)),
    )
    .unwrap()
    .with_sda(gpio12)
    .with_scl(gpio11);

    SYSTEM_I2C.init(RefCell::new(i2c))
}

/// Creates a lightweight handle to the shared internal I2C bus.
///
/// `RefCellDevice` is intentionally used here because all current I2C users
/// (board init, touch polling, codec init) execute on the same Embassy executor
/// thread and each transaction is blocking/no-await. This avoids wrapping every
/// I2C transaction in a global critical section.
pub fn device(bus: SystemI2cBus) -> SystemI2cDevice {
    RefCellDevice::new(bus)
}
