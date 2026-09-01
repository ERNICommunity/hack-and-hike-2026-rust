use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex,
    mutex::Mutex,
};
use esp_hal::{
    Blocking,
    i2c::master::{Config as I2cConfig, I2c},
    peripherals::{GPIO11, GPIO12, I2C0},
    time::Rate,
};
use static_cell::StaticCell;

pub type SystemI2c = I2c<'static, Blocking>;
pub type SystemI2cMutex = Mutex<CriticalSectionRawMutex, SystemI2c>;
pub type SystemI2cBus = &'static SystemI2cMutex;

static SYSTEM_I2C: StaticCell<SystemI2cMutex> = StaticCell::new();

/// Initializes the one physical CoreS3-Lite internal I2C controller.
///
/// Every subsystem receives this same bus mutex rather than owning an I2C
/// device wrapper. The async mutex is cross-core safe. Its raw critical-section
/// mutex is used only while updating mutex state; it is not held for the whole
/// blocking I2C transaction.
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

    SYSTEM_I2C.init(Mutex::new(i2c))
}
