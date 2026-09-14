//! Driver for the BMI270 accelerometer and gyroscope, and for the BMM150
//! magnetometer connected behind it.
//!
//! The BMM150 is not on the board's I2C bus: it hangs off the BMI270's
//! *auxiliary* interface. To configure it, the BMI270 forwards register
//! reads and writes (manual mode). Once running, the BMI270 reads the
//! magnetometer by itself and exposes its data next to its own, so one
//! 23-byte I2C read returns magnetometer, accelerometer, gyroscope and
//! timestamp together.
//!
//! The BMI270 also needs a configuration blob uploaded at every power-up
//! before it measures anything; see [`config`].

mod config;

use core::fmt;

use embassy_time::{Duration, Timer};
use esp_hal::i2c::master::Error as I2cError;

use crate::board::i2c::SystemI2cBus;

use hack_and_hike_core::imu::bmm150::Trim;

/// I2C address of the BMI270 on the board (0x69: its SDO pin is tied high).
const BMI270_ADDR: u8 = 0x69;
/// The ID a BMI270 reports in `REG_CHIP_ID`; anything else is a different chip.
const BMI270_CHIP_ID: u8 = 0x24;

/// Chip identification register, read once to check the part is a BMI270.
const REG_CHIP_ID: u8 = 0x00;
/// Status flags; bit 2 (`AUX_BUSY`) is set while a forwarded magnetometer
/// register access is still running.
const REG_STATUS: u8 = 0x03;
/// First data register (DATA_0). The 23-byte burst read starts here:
/// 8 magnetometer bytes, 6 accelerometer, 6 gyroscope, 3 timestamp.
/// In manual auxiliary mode it also holds the byte a forwarded read returned.
const REG_AUX_X_LSB: u8 = 0x04;
/// Internal status; its low nibble reports whether the uploaded
/// configuration blob was accepted (`CONFIG_LOAD_OK`).
const REG_INTERNAL_STATUS: u8 = 0x21;
/// Accelerometer output data rate and filter settings.
const REG_ACC_CONF: u8 = 0x40;
/// Accelerometer measurement range.
const REG_ACC_RANGE: u8 = 0x41;
/// Gyroscope output data rate and filter settings.
const REG_GYR_CONF: u8 = 0x42;
/// Gyroscope measurement range.
const REG_GYR_RANGE: u8 = 0x43;
/// Rate at which the BMI270 reads the auxiliary magnetometer by itself.
const REG_AUX_CONF: u8 = 0x44;
/// I2C address of the device on the auxiliary bus, stored shifted left by
/// one bit.
const REG_AUX_DEV_ID: u8 = 0x4B;
/// Auxiliary interface mode: manual (register access forwarded on request)
/// or automatic (periodic data reads), plus the read burst length.
const REG_AUX_IF_CONF: u8 = 0x4C;
/// Magnetometer register to read through the auxiliary interface. Writing
/// it starts a manual read; in automatic mode it is the first register
/// of every periodic read.
const REG_AUX_RD_ADDR: u8 = 0x4D;
/// Magnetometer register to write through the auxiliary interface. Writing
/// it starts the write of the byte in `REG_AUX_WR_DATA`.
const REG_AUX_WR_ADDR: u8 = 0x4E;
/// Byte to write to the magnetometer register named in `REG_AUX_WR_ADDR`.
const REG_AUX_WR_DATA: u8 = 0x4F;
/// Configuration upload control: 0 while uploading the blob, 1 to make the
/// chip load and check it.
const REG_INIT_CTRL: u8 = 0x59;
/// Upload position of the next configuration chunk. This and the following
/// register together hold a word address (low nibble, then high byte).
const REG_INIT_ADDR_0: u8 = 0x5B;
/// Configuration data register: bytes written here land in the chip's
/// configuration memory at the position set through `REG_INIT_ADDR_0`.
const REG_INIT_DATA: u8 = 0x5E;
/// Auxiliary interface pad trim, used here to select the pull-up resistors
/// on the auxiliary bus lines.
const REG_AUX_IF_TRIM: u8 = 0x68;
/// Interface configuration; bit 5 (0x20) enables the auxiliary interface.
const REG_IF_CONF: u8 = 0x6B;
/// Power configuration. Writing 0 turns advanced power saving off, which
/// the configuration upload and auxiliary access need.
const REG_PWR_CONF: u8 = 0x7C;
/// Enables the individual sensors: auxiliary, gyroscope, accelerometer and
/// temperature (bits 0 to 3).
const REG_PWR_CTRL: u8 = 0x7D;
/// Command register; accepts one-shot commands such as `CMD_SOFT_RESET`.
const REG_CMD: u8 = 0x7E;

// BMM150 registers reached through the BMI270 auxiliary interface.
/// The BMM150's I2C address on the auxiliary bus, unshifted.
const BMM150_ADDRESS: u8 = 0x10;
/// The ID a BMM150 reports in `BMM150_REG_CHIP_ID`.
const BMM150_CHIP_ID: u8 = 0x32;
/// BMM150 chip identification register. It only answers once the chip is
/// powered through `BMM150_REG_POWER_CONTROL`.
const BMM150_REG_CHIP_ID: u8 = 0x40;
/// First BMM150 data register: X, Y, Z and the Hall resistance follow
/// in 8 bytes, which is the frame the BMI270 copies in automatic mode.
const BMM150_REG_DATA_X_LSB: u8 = 0x42;
/// BMM150 power control: power bit and soft-reset bits.
const BMM150_REG_POWER_CONTROL: u8 = 0x4B;
/// BMM150 operation mode (normal, forced, sleep) and output data rate.
const BMM150_REG_OP_MODE: u8 = 0x4C;
/// Number of repetitions per X/Y measurement; more repetitions average away
/// noise at the cost of current.
const BMM150_REG_REP_XY: u8 = 0x51;
/// Number of repetitions per Z measurement.
const BMM150_REG_REP_Z: u8 = 0x52;
/// First factory trim register (`dig_x1`, `dig_y1`), read in 2 bytes.
const BMM150_DIG_X1: u8 = 0x5D;
/// Second trim block (`dig_z4`, `dig_x2`, `dig_y2`), read in 4 bytes.
const BMM150_DIG_Z4_LSB: u8 = 0x62;
/// Third trim block (`dig_z2` through `dig_xy1`), read in 10 bytes.
const BMM150_DIG_Z2_LSB: u8 = 0x68;
/// Power control value: soft reset plus power on, which leaves the BMM150
/// in sleep mode ready to be configured.
const BMM150_SOFT_RESET_AND_POWER: u8 = 0x83;
/// Operation mode value: normal mode with a 30 Hz output data rate.
const BMM150_NORMAL_30HZ: u8 = 0x38;
/// X/Y repetitions: how many measurements the BMM150 averages into one
/// X/Y sample. More repetitions lower the noise and raise the current.
const BMM150_REP_XY_REGULAR: u8 = 0x04;
/// Z repetitions, as for `BMM150_REP_XY_REGULAR`.
const BMM150_REP_Z_REGULAR: u8 = 0x07;

/// Soft reset command for `REG_CMD`: the chip restarts as after power-up,
/// so the configuration blob must be uploaded again.
const CMD_SOFT_RESET: u8 = 0xB6;
/// Low-nibble value of `REG_INTERNAL_STATUS` once the configuration blob
/// was accepted.
const CONFIG_LOAD_OK: u8 = 0x01;
/// Bit in `REG_STATUS` that is set while a forwarded auxiliary access runs.
const AUX_BUSY: u8 = 1 << 2;

// Acceleration stays at 100 Hz. Gyro runs at 400 Hz, performance filtering,
// normal bandwidth: lower group delay and much higher bandwidth during fast yaw
// without increasing host I2C traffic. Range remains +/-2000 dps.
/// `REG_ACC_CONF` value: 100 Hz, performance filter mode.
const ACC_CONF_100HZ: u8 = 0xA8;
/// `REG_GYR_CONF` value: 400 Hz, performance filter mode, normal bandwidth.
const GYR_CONF_400HZ: u8 = 0xAA;
/// `REG_ACC_RANGE` value: ±4 g full scale.
const ACC_RANGE_4G: u8 = 0x01;
/// `REG_GYR_RANGE` value: ±2000 degrees per second full scale.
const GYR_RANGE_2000DPS: u8 = 0x00;
/// `REG_PWR_CTRL` value: accelerometer and gyroscope on, auxiliary
/// interface off.
const PWR_CTRL_ACC_GYR: u8 = 0x06;
/// `REG_PWR_CTRL` value: auxiliary interface, gyroscope, accelerometer and
/// temperature sensor on.
const PWR_CTRL_ACC_GYR_AUX: u8 = 0x0F;

// Poll the 30 Hz BMM150 at 100 Hz inside BMI270. This does not add host I2C
// traffic, but halves worst-case AUX pickup latency compared with 50 Hz.
/// `REG_AUX_CONF` value: read the magnetometer 100 times per second.
const AUX_CONF_100HZ: u8 = 0x48;
/// `REG_AUX_IF_CONF` value: automatic mode, reading 8 bytes per cycle, so
/// the magnetometer frame appears in the data registers without host help.
const AUX_IF_DATA_MODE_8_BYTES: u8 = 0x4F;
/// `REG_AUX_IF_CONF` value: manual mode, where each register access is
/// forwarded when the host asks for it.
const AUX_IF_MANUAL_MODE: u8 = 0x80;
/// `REG_AUX_IF_TRIM` value selecting the 2 kΩ pull-ups on the auxiliary bus.
const AUX_IF_TRIM_2K_PULLUP: u8 = 0x03;

/// Accelerometer scale: g per raw count at ±4 g (a signed 16-bit reading
/// covers the full range).
const ACC_G_PER_LSB: f32 = 4.0 / 32768.0;
/// Gyroscope scale: degrees per second per raw count at ±2000 °/s.
const GYR_DPS_PER_LSB: f32 = 2000.0 / 32768.0;
/// Settle time after enabling the accelerometer and gyroscope.
const SENSOR_STARTUP: Duration = Duration::from_millis(50);
/// Bytes of the configuration blob written per I2C transaction.
const CONFIG_CHUNK_BYTES: usize = 32;

/// Why a BMI270 or BMM150 operation failed.
#[derive(Clone, Copy, Debug)]
pub(super) enum Error {
    /// The I2C transaction failed, usually a missing acknowledge.
    Bus(I2cError),
    /// The chip at the BMI270 address reported this ID instead.
    ChipId(u8),
    /// The BMI270 did not accept its configuration blob.
    ConfigStatus(u8),
    /// The auxiliary (magnetometer) interface stayed busy.
    AuxBusy,
    /// The chip on the auxiliary bus reported this ID instead of a BMM150.
    BmmChipId(u8),
}

impl From<I2cError> for Error {
    fn from(error: I2cError) -> Self {
        Self::Bus(error)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bus(error) => write!(f, "I2C error {error:?}"),
            Self::ChipId(id) => write!(f, "unexpected BMI270 chip id 0x{id:02x}"),
            Self::ConfigStatus(status) => write!(f, "config load status 0x{status:02x}"),
            Self::AuxBusy => write!(f, "auxiliary interface busy"),
            Self::BmmChipId(id) => write!(f, "unexpected BMM150 chip id 0x{id:02x}"),
        }
    }
}

/// One reading, in the BMI270's own axes (the body frame).
#[derive(Clone, Copy)]
pub(super) struct RawSample {
    /// Acceleration in g.
    pub(super) accel_g: [f32; 3],
    /// Rotation rate in degrees per second, before bias correction.
    pub(super) gyro_dps: [f32; 3],
    /// The magnetometer's raw data frame, decoded by `bmm150::compensate`.
    pub(super) mag_data: [u8; 8],
    /// The BMI270's 24-bit timestamp of this sample.
    pub(super) sensor_time: u32,
}

/// The BMI270 on the shared system bus.
pub(super) struct Bmi270 {
    /// Shared I2C bus; each register access locks it only for its own
    /// transaction, so other drivers can interleave.
    bus: SystemI2cBus,
}

impl Bmi270 {
    /// A driver for the BMI270 on `bus`. Nothing is sent until
    /// [`Bmi270::initialize`].
    pub(super) const fn new(bus: SystemI2cBus) -> Self {
        Self { bus }
    }

    /// Write one BMI270 register.
    async fn write_register(&self, register: u8, value: u8) -> Result<(), Error> {
        let mut i2c = self.bus.lock().await;
        i2c.write_async(BMI270_ADDR, &[register, value]).await?;
        Ok(())
    }

    /// Read one BMI270 register.
    async fn read_register(&self, register: u8) -> Result<u8, Error> {
        let mut value = [0u8; 1];
        let mut i2c = self.bus.lock().await;
        i2c.write_read_async(BMI270_ADDR, &[register], &mut value)
            .await?;
        Ok(value[0])
    }

    /// Upload the configuration blob in chunks, each preceded by its
    /// position in the chip's configuration memory.
    async fn upload_config(&self) -> Result<(), Error> {
        let chunks = config::MAXIMUM_FIFO_CONFIG.chunks(CONFIG_CHUNK_BYTES);
        for (index, chunk) in chunks.enumerate() {
            let byte_offset = index * CONFIG_CHUNK_BYTES;
            // The init address is a word address split into a low nibble and
            // a high byte across two registers.
            let word_address = byte_offset >> 1;
            let address = [
                REG_INIT_ADDR_0,
                u8::try_from(word_address & 0x0F).expect("a nibble fits u8"),
                u8::try_from(word_address >> 4).expect("BMI270 config fits its address space"),
            ];

            let mut packet = [0u8; 1 + CONFIG_CHUNK_BYTES];
            packet[0] = REG_INIT_DATA;
            packet[1..1 + chunk.len()].copy_from_slice(chunk);

            let mut i2c = self.bus.lock().await;
            i2c.write_async(BMI270_ADDR, &address).await?;
            i2c.write_async(BMI270_ADDR, &packet[..1 + chunk.len()])
                .await?;
        }

        Ok(())
    }

    /// Reset the BMI270, upload its configuration and start the
    /// accelerometer (100 Hz, ±4 g) and gyroscope (400 Hz, ±2000 °/s).
    pub(super) async fn initialize(&self) -> Result<(), Error> {
        let chip_id = self.read_register(REG_CHIP_ID).await?;
        if chip_id != BMI270_CHIP_ID {
            return Err(Error::ChipId(chip_id));
        }

        self.write_register(REG_CMD, CMD_SOFT_RESET).await?;
        Timer::after(Duration::from_millis(2)).await;

        self.write_register(REG_PWR_CONF, 0x00).await?;
        Timer::after(Duration::from_millis(1)).await;
        self.write_register(REG_INIT_CTRL, 0x00).await?;
        self.upload_config().await?;
        Timer::after(Duration::from_millis(1)).await;
        self.write_register(REG_INIT_CTRL, 0x01).await?;

        let mut last_status = 0u8;
        let mut initialized = false;
        for _ in 0..20 {
            Timer::after(Duration::from_millis(1)).await;
            last_status = self.read_register(REG_INTERNAL_STATUS).await?;
            if last_status & 0x0F == CONFIG_LOAD_OK {
                initialized = true;
                break;
            }
        }
        if !initialized {
            return Err(Error::ConfigStatus(last_status));
        }

        self.write_register(REG_ACC_CONF, ACC_CONF_100HZ).await?;
        self.write_register(REG_ACC_RANGE, ACC_RANGE_4G).await?;
        self.write_register(REG_GYR_CONF, GYR_CONF_400HZ).await?;
        self.write_register(REG_GYR_RANGE, GYR_RANGE_2000DPS)
            .await?;
        self.write_register(REG_PWR_CTRL, PWR_CTRL_ACC_GYR).await?;
        Timer::after(SENSOR_STARTUP).await;

        Ok(())
    }

    /// Wait a few milliseconds for the auxiliary interface to finish a
    /// forwarded register access.
    async fn wait_aux_idle(&self) -> Result<(), Error> {
        for _ in 0..8 {
            if self.read_register(REG_STATUS).await? & AUX_BUSY == 0 {
                return Ok(());
            }
            Timer::after(Duration::from_millis(1)).await;
        }
        Err(Error::AuxBusy)
    }

    /// Write one BMM150 register through the auxiliary interface.
    async fn aux_write_register(&self, register: u8, value: u8) -> Result<(), Error> {
        self.write_register(REG_AUX_WR_DATA, value).await?;
        self.write_register(REG_AUX_WR_ADDR, register).await?;
        self.wait_aux_idle().await
    }

    /// Read one BMM150 register through the auxiliary interface.
    async fn aux_read_register(&self, register: u8) -> Result<u8, Error> {
        self.write_register(REG_AUX_IF_CONF, AUX_IF_MANUAL_MODE)
            .await?;
        self.write_register(REG_AUX_RD_ADDR, register).await?;
        self.wait_aux_idle().await?;
        self.read_register(REG_AUX_X_LSB).await
    }

    /// Read `N` consecutive BMM150 registers starting at `first`.
    async fn aux_read_array<const N: usize>(&self, first: u8) -> Result<[u8; N], Error> {
        let mut result = [0u8; N];
        let mut register = first;
        for byte in &mut result {
            *byte = self.aux_read_register(register).await?;
            register = register.wrapping_add(1);
        }
        Ok(result)
    }

    /// Wake the BMM150 behind the auxiliary interface, read its factory trim
    /// values, start it at 30 Hz and let the BMI270 read it automatically.
    pub(super) async fn initialize_bmm150(&self) -> Result<Trim, Error> {
        self.write_register(REG_IF_CONF, 0x20).await?;
        self.write_register(REG_PWR_CONF, 0x00).await?;
        self.write_register(REG_PWR_CTRL, PWR_CTRL_ACC_GYR).await?;
        self.write_register(REG_AUX_IF_TRIM, AUX_IF_TRIM_2K_PULLUP)
            .await?;
        self.write_register(REG_AUX_IF_CONF, AUX_IF_MANUAL_MODE)
            .await?;
        self.write_register(REG_AUX_DEV_ID, BMM150_ADDRESS << 1)
            .await?;

        self.aux_write_register(BMM150_REG_POWER_CONTROL, BMM150_SOFT_RESET_AND_POWER)
            .await?;
        Timer::after(Duration::from_millis(5)).await;

        let chip_id = self.aux_read_register(BMM150_REG_CHIP_ID).await?;
        if chip_id != BMM150_CHIP_ID {
            return Err(Error::BmmChipId(chip_id));
        }

        let x1_y1 = self.aux_read_array::<2>(BMM150_DIG_X1).await?;
        let z4_x2_y2 = self.aux_read_array::<4>(BMM150_DIG_Z4_LSB).await?;
        let z2_to_xy1 = self.aux_read_array::<10>(BMM150_DIG_Z2_LSB).await?;
        let trim = Trim::from_registers(x1_y1, z4_x2_y2, z2_to_xy1);

        self.aux_write_register(BMM150_REG_REP_XY, BMM150_REP_XY_REGULAR)
            .await?;
        self.aux_write_register(BMM150_REG_REP_Z, BMM150_REP_Z_REGULAR)
            .await?;
        self.aux_write_register(BMM150_REG_OP_MODE, BMM150_NORMAL_30HZ)
            .await?;

        self.write_register(REG_AUX_CONF, AUX_CONF_100HZ).await?;
        self.write_register(REG_AUX_IF_CONF, AUX_IF_DATA_MODE_8_BYTES)
            .await?;
        self.write_register(REG_AUX_RD_ADDR, BMM150_REG_DATA_X_LSB)
            .await?;
        self.write_register(REG_PWR_CTRL, PWR_CTRL_ACC_GYR_AUX)
            .await?;
        Timer::after(Duration::from_millis(10)).await;

        Ok(trim)
    }

    /// Turn the auxiliary interface off again after a failed BMM150 start.
    /// Failure is ignored: the accelerometer and gyroscope keep working either
    /// way, and the next retry repeats the whole sequence.
    pub(super) async fn disable_aux(&self) {
        if self
            .write_register(REG_PWR_CTRL, PWR_CTRL_ACC_GYR)
            .await
            .is_err()
        {
            log::debug!("BMI270 auxiliary interface could not be disabled");
        }
    }

    /// Read the newest magnetometer frame, acceleration, rotation and
    /// timestamp in one I2C transaction.
    pub(super) async fn read_sample(&self) -> Result<RawSample, Error> {
        // DATA_0..DATA_19 followed by the three SENSORTIME bytes. Bosch defines
        // sensor time as shadowed at the start of a burst that begins in the data
        // registers, so this gives a coherent 25.6 kHz timestamp at no extra I2C
        // transaction cost.
        let mut bytes = [0u8; 23];
        let mut i2c = self.bus.lock().await;
        i2c.write_read_async(BMI270_ADDR, &[REG_AUX_X_LSB], &mut bytes)
            .await?;
        drop(i2c);

        let mag_data = bytes[..8].try_into().expect("eight magnetometer bytes");

        let acc = [
            i16::from_le_bytes([bytes[8], bytes[9]]),
            i16::from_le_bytes([bytes[10], bytes[11]]),
            i16::from_le_bytes([bytes[12], bytes[13]]),
        ];
        let gyr = [
            i16::from_le_bytes([bytes[14], bytes[15]]),
            i16::from_le_bytes([bytes[16], bytes[17]]),
            i16::from_le_bytes([bytes[18], bytes[19]]),
        ];
        let sensor_time =
            u32::from(bytes[20]) | (u32::from(bytes[21]) << 8) | (u32::from(bytes[22]) << 16);

        Ok(RawSample {
            accel_g: [
                f32::from(acc[0]) * ACC_G_PER_LSB,
                f32::from(acc[1]) * ACC_G_PER_LSB,
                f32::from(acc[2]) * ACC_G_PER_LSB,
            ],
            gyro_dps: [
                f32::from(gyr[0]) * GYR_DPS_PER_LSB,
                f32::from(gyr[1]) * GYR_DPS_PER_LSB,
                f32::from(gyr[2]) * GYR_DPS_PER_LSB,
            ],
            mag_data,
            sensor_time,
        })
    }
}
