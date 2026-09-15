//! Driver for the BMI270 accelerometer and gyroscope, and for the BMM150
//! magnetometer that is connected to the BMI270. Both chips are made by
//! Bosch Sensortec.
//!
//! The BMM150 is not on the board's I2C bus (the two-wire bus for sensors).
//! It is connected to the *auxiliary* (AUX) interface of the BMI270, a
//! second, private I2C bus. During setup, the BMI270 forwards each register
//! read and write to the BMM150 (manual mode). After setup, the BMI270 reads
//! the magnetometer by itself (automatic mode). It puts the magnetometer
//! data in front of its own data. So one 23-byte I2C read returns the
//! magnetometer, accelerometer, gyroscope and timestamp data together.
//!
//! After every power-up or reset, the host must upload a configuration file
//! (the "blob") to the BMI270. The BMI270 measures nothing before that. See
//! [`config`].
//!
//! The register names follow the BMI270 and BMM150 datasheets.

mod config;

use core::fmt;

use embassy_time::{Duration, Timer};
use esp_hal::i2c::master::Error as I2cError;

use crate::board::i2c::SystemI2cBus;

use hack_and_hike_core::imu::bmm150::Trim;

/// I2C address of the BMI270 on the board. It is 0x69 because the SDO pin of
/// the chip is connected to the supply voltage (high).
const BMI270_ADDR: u8 = 0x69;
/// The ID that a BMI270 reports in `REG_CHIP_ID`. Any other value means a
/// different chip.
const BMI270_CHIP_ID: u8 = 0x24;

/// Chip ID register. The driver reads it once to check that the chip is a
/// BMI270.
const REG_CHIP_ID: u8 = 0x00;
/// Status flags. Bit 2 (`AUX_BUSY`) is set while a forwarded magnetometer
/// register access is still running.
const REG_STATUS: u8 = 0x03;
/// First data register (DATA_0). The 23-byte read in
/// [`Bmi270::read_sample`] starts here. It returns 8 magnetometer bytes, 6
/// accelerometer bytes, 6 gyroscope bytes and 3 timestamp bytes. In manual
/// AUX mode, this register holds the byte from a forwarded read.
const REG_AUX_X_LSB: u8 = 0x04;
/// Internal status. Its low 4 bits show whether the chip accepted the
/// uploaded configuration blob (`CONFIG_LOAD_OK`).
const REG_INTERNAL_STATUS: u8 = 0x21;
/// Accelerometer output data rate (ODR) and filter settings.
const REG_ACC_CONF: u8 = 0x40;
/// Accelerometer measurement range.
const REG_ACC_RANGE: u8 = 0x41;
/// Gyroscope output data rate (ODR) and filter settings.
const REG_GYR_CONF: u8 = 0x42;
/// Gyroscope measurement range.
const REG_GYR_RANGE: u8 = 0x43;
/// AUX configuration: how often the BMI270 reads the magnetometer by
/// itself in automatic mode.
const REG_AUX_CONF: u8 = 0x44;
/// I2C address of the chip on the AUX bus. The register stores the address
/// shifted left by one bit.
const REG_AUX_DEV_ID: u8 = 0x4B;
/// AUX interface mode and read length. In manual mode, the BMI270 forwards
/// a register access when the host asks. In automatic mode, it reads the
/// data regularly.
const REG_AUX_IF_CONF: u8 = 0x4C;
/// Magnetometer register to read through the AUX interface. In manual mode,
/// a write to this register starts one read. In automatic mode, it is the
/// first register of every regular read.
const REG_AUX_RD_ADDR: u8 = 0x4D;
/// Magnetometer register to write through the AUX interface. A write to
/// this register starts the write of the byte in `REG_AUX_WR_DATA`.
const REG_AUX_WR_ADDR: u8 = 0x4E;
/// Byte to write to the magnetometer register in `REG_AUX_WR_ADDR`.
const REG_AUX_WR_DATA: u8 = 0x4F;
/// Control of the configuration upload: 0 during the upload of the blob, 1
/// to make the chip load and check the blob.
const REG_INIT_CTRL: u8 = 0x59;
/// Upload position of the next configuration chunk. This register and the
/// next one (0x5C) hold a word address: the low 4 bits here, the high 8
/// bits in 0x5C.
const REG_INIT_ADDR_0: u8 = 0x5B;
/// Configuration data register. The chip stores bytes written here in its
/// configuration memory, at the position from `REG_INIT_ADDR_0`.
const REG_INIT_DATA: u8 = 0x5E;
/// AUX interface pad trim. Here it selects the pull-up resistors on the
/// AUX bus lines.
const REG_AUX_IF_TRIM: u8 = 0x68;
/// Interface configuration. Bit 5 (0x20) turns on the AUX interface.
const REG_IF_CONF: u8 = 0x6B;
/// Power configuration. Writing 0 turns off the advanced power saving. The
/// configuration upload and the AUX access need this.
const REG_PWR_CONF: u8 = 0x7C;
/// Turns on the individual sensors: bit 0 AUX, bit 1 gyroscope, bit 2
/// accelerometer, bit 3 temperature sensor.
const REG_PWR_CTRL: u8 = 0x7D;
/// Command register. It accepts single commands such as `CMD_SOFT_RESET`.
const REG_CMD: u8 = 0x7E;

// BMM150 values and registers. The driver reaches the registers through
// the BMI270 AUX interface.
/// The BMM150's I2C address on the AUX bus, not shifted.
const BMM150_ADDRESS: u8 = 0x10;
/// The ID that a BMM150 reports in `BMM150_REG_CHIP_ID`.
const BMM150_CHIP_ID: u8 = 0x32;
/// BMM150 chip ID register. It answers only after the chip is powered on
/// through `BMM150_REG_POWER_CONTROL`.
const BMM150_REG_CHIP_ID: u8 = 0x40;
/// First BMM150 data register. The next 8 bytes hold X, Y, Z and the Hall
/// resistance (RHALL). In automatic mode, the BMI270 copies these 8 bytes.
const BMM150_REG_DATA_X_LSB: u8 = 0x42;
/// BMM150 power control: the power bit and the soft reset bits.
const BMM150_REG_POWER_CONTROL: u8 = 0x4B;
/// BMM150 operation mode (normal, forced, sleep) and output data rate.
const BMM150_REG_OP_MODE: u8 = 0x4C;
/// Repetitions for each X and Y measurement. More repetitions give less
/// noise but use more current.
const BMM150_REG_REP_XY: u8 = 0x51;
/// Repetitions for each Z measurement.
const BMM150_REG_REP_Z: u8 = 0x52;
/// First factory trim register (`dig_x1`, `dig_y1`), read as 2 bytes.
const BMM150_DIG_X1: u8 = 0x5D;
/// Second trim block (`dig_z4`, `dig_x2`, `dig_y2`), read as 4 bytes.
const BMM150_DIG_Z4_LSB: u8 = 0x62;
/// Third trim block (`dig_z2` to `dig_xy1`), read as 10 bytes.
const BMM150_DIG_Z2_LSB: u8 = 0x68;
/// Power control value: soft reset and power on. After this, the BMM150 is
/// in sleep mode and ready for its configuration.
const BMM150_SOFT_RESET_AND_POWER: u8 = 0x83;
/// Operation mode value: normal mode with an output data rate of 30 Hz.
const BMM150_NORMAL_30HZ: u8 = 0x38;
/// `BMM150_REG_REP_XY` value for the "regular" preset. It sets how many
/// measurements the BMM150 averages into one X or Y value. The datasheet
/// shows how the register value maps to the number of repetitions.
const BMM150_REP_XY_REGULAR: u8 = 0x04;
/// `BMM150_REG_REP_Z` value for the "regular" preset. It sets how many
/// measurements the BMM150 averages into one Z value.
const BMM150_REP_Z_REGULAR: u8 = 0x07;

/// Soft reset command for `REG_CMD`. The chip restarts as after power-up, so
/// the configuration blob must be uploaded again.
const CMD_SOFT_RESET: u8 = 0xB6;
/// Value of the low 4 bits of `REG_INTERNAL_STATUS` when the chip accepted
/// the configuration blob.
const CONFIG_LOAD_OK: u8 = 0x01;
/// Bit in `REG_STATUS` that is set while a forwarded AUX access runs.
const AUX_BUSY: u8 = 1 << 2;

// The accelerometer measures 100 times per second. The gyroscope measures
// 400 times per second, with the performance filter and normal bandwidth.
// Compared with a lower rate, its filter adds less delay and lets faster
// rotation through. The host still reads only 100 times per second, so the
// I2C traffic does not grow. The gyroscope range stays ±2000 degrees per
// second (dps).
/// `REG_ACC_CONF` value: 100 Hz, performance filter mode, normal bandwidth.
const ACC_CONF_100HZ: u8 = 0xA8;
/// `REG_GYR_CONF` value: 400 Hz, performance filter mode, normal bandwidth.
const GYR_CONF_400HZ: u8 = 0xAA;
/// `REG_ACC_RANGE` value: ±4 g full scale.
const ACC_RANGE_4G: u8 = 0x01;
/// `REG_GYR_RANGE` value: ±2000 degrees per second full scale.
const GYR_RANGE_2000DPS: u8 = 0x00;
/// `REG_PWR_CTRL` value: accelerometer and gyroscope on, AUX interface and
/// temperature sensor off.
const PWR_CTRL_ACC_GYR: u8 = 0x06;
/// `REG_PWR_CTRL` value: AUX interface, gyroscope, accelerometer and
/// temperature sensor on.
const PWR_CTRL_ACC_GYR_AUX: u8 = 0x0F;

// The BMM150 measures 30 times per second. The BMI270 reads it 100 times per
// second. This adds no I2C traffic for the host. Compared with 50 reads per
// second, a new measurement reaches the data registers at most 10 ms late
// instead of 20 ms.
/// `REG_AUX_CONF` value: read the magnetometer 100 times per second (low 4
/// bits 0x8). The high 4 bits (0x4) set the read offset.
const AUX_CONF_100HZ: u8 = 0x48;
/// `REG_AUX_IF_CONF` value: automatic mode, 8 bytes per read. The
/// magnetometer frame then appears in the data registers without help from
/// the host.
const AUX_IF_DATA_MODE_8_BYTES: u8 = 0x4F;
/// `REG_AUX_IF_CONF` value: manual mode. The BMI270 forwards each register
/// access when the host asks for it.
const AUX_IF_MANUAL_MODE: u8 = 0x80;
/// `REG_AUX_IF_TRIM` value that selects the 2 kΩ pull-up resistors on the
/// AUX bus.
const AUX_IF_TRIM_2K_PULLUP: u8 = 0x03;

/// Accelerometer scale: g per raw count (LSB) at ±4 g. The signed 16-bit
/// value covers the full range.
const ACC_G_PER_LSB: f32 = 4.0 / 32768.0;
/// Gyroscope scale: degrees per second per raw count (LSB) at ±2000 °/s.
const GYR_DPS_PER_LSB: f32 = 2000.0 / 32768.0;
/// Wait after turning on the accelerometer and the gyroscope, so that their
/// first values are valid.
const SENSOR_STARTUP: Duration = Duration::from_millis(50);
/// Bytes of the configuration blob in each I2C write.
const CONFIG_CHUNK_BYTES: usize = 32;

/// Why a BMI270 or BMM150 operation failed.
#[derive(Clone, Copy, Debug)]
pub(super) enum Error {
    /// The I2C transaction failed. Usually the chip did not acknowledge.
    Bus(I2cError),
    /// The chip at the BMI270 address reported this ID instead of the
    /// BMI270 ID.
    ChipId(u8),
    /// The BMI270 did not accept its configuration blob. The value is the
    /// last `REG_INTERNAL_STATUS`.
    ConfigStatus(u8),
    /// The AUX (magnetometer) interface was still busy after about 8 ms.
    AuxBusy,
    /// The chip on the AUX bus reported this ID instead of the BMM150 ID.
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
    /// Acceleration in g (1 g is about 9.81 m/s²).
    pub(super) accel_g: [f32; 3],
    /// Rotation speed in degrees per second, before the gyroscope offset is
    /// removed.
    pub(super) gyro_dps: [f32; 3],
    /// The magnetometer's raw data frame, in the BMM150's own axes.
    /// `bmm150::compensate` decodes it.
    pub(super) mag_data: [u8; 8],
    /// The BMI270's 24-bit timestamp of this read, in ticks of 1/25,600 s.
    pub(super) sensor_time: u32,
}

/// The BMI270 on the shared system I2C bus.
pub(super) struct Bmi270 {
    /// The shared I2C bus. Each register access locks the bus only for its
    /// own transaction. So other drivers can use the bus between two
    /// accesses.
    bus: SystemI2cBus,
}

impl Bmi270 {
    /// A driver for the BMI270 on `bus`. It sends nothing before
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

    /// Upload the configuration blob in chunks. Before each chunk, write its
    /// position in the chip's configuration memory.
    ///
    /// Each chunk locks the bus for its two writes, so no other driver can
    /// write between the position and the data.
    async fn upload_config(&self) -> Result<(), Error> {
        let chunks = config::MAXIMUM_FIFO_CONFIG.chunks(CONFIG_CHUNK_BYTES);
        for (index, chunk) in chunks.enumerate() {
            let byte_offset = index * CONFIG_CHUNK_BYTES;
            // The position is a word address (one word is 2 bytes). One
            // write fills both address registers: the low 4 bits go to
            // `REG_INIT_ADDR_0`, the remaining bits to the next register.
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
    /// accelerometer (100 Hz, ±4 g) and the gyroscope (400 Hz, ±2000 °/s).
    ///
    /// The AUX interface stays off. [`Bmi270::initialize_bmm150`] turns it
    /// on.
    ///
    /// # Errors
    ///
    /// - [`Error::Bus`] when an I2C transaction fails.
    /// - [`Error::ChipId`] when the chip is not a BMI270.
    /// - [`Error::ConfigStatus`] when the chip does not accept the
    ///   configuration blob within about 20 ms.
    pub(super) async fn initialize(&self) -> Result<(), Error> {
        let chip_id = self.read_register(REG_CHIP_ID).await?;
        if chip_id != BMI270_CHIP_ID {
            return Err(Error::ChipId(chip_id));
        }

        self.write_register(REG_CMD, CMD_SOFT_RESET).await?;
        Timer::after(Duration::from_millis(2)).await;

        // Turn off the advanced power saving before the upload, and give the
        // chip a short time to wake up.
        self.write_register(REG_PWR_CONF, 0x00).await?;
        Timer::after(Duration::from_millis(1)).await;
        self.write_register(REG_INIT_CTRL, 0x00).await?;
        self.upload_config().await?;
        Timer::after(Duration::from_millis(1)).await;
        self.write_register(REG_INIT_CTRL, 0x01).await?;

        // Check about every millisecond, up to 20 times, whether the chip
        // accepted the blob.
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

    /// Wait until the AUX interface has finished a forwarded register access.
    ///
    /// # Errors
    ///
    /// [`Error::AuxBusy`] when the interface is still busy after 8 checks,
    /// about 8 ms. [`Error::Bus`] when an I2C transaction fails.
    async fn wait_aux_idle(&self) -> Result<(), Error> {
        for _ in 0..8 {
            if self.read_register(REG_STATUS).await? & AUX_BUSY == 0 {
                return Ok(());
            }
            Timer::after(Duration::from_millis(1)).await;
        }
        Err(Error::AuxBusy)
    }

    /// Write one BMM150 register through the AUX interface.
    ///
    /// The AUX interface must already be in manual mode.
    async fn aux_write_register(&self, register: u8, value: u8) -> Result<(), Error> {
        self.write_register(REG_AUX_WR_DATA, value).await?;
        self.write_register(REG_AUX_WR_ADDR, register).await?;
        self.wait_aux_idle().await
    }

    /// Read one BMM150 register through the AUX interface. This function
    /// sets manual mode first.
    async fn aux_read_register(&self, register: u8) -> Result<u8, Error> {
        self.write_register(REG_AUX_IF_CONF, AUX_IF_MANUAL_MODE)
            .await?;
        self.write_register(REG_AUX_RD_ADDR, register).await?;
        self.wait_aux_idle().await?;
        self.read_register(REG_AUX_X_LSB).await
    }

    /// Read `N` BMM150 registers in a row, starting at `first`. Each register
    /// is a separate forwarded read.
    async fn aux_read_array<const N: usize>(&self, first: u8) -> Result<[u8; N], Error> {
        let mut result = [0u8; N];
        let mut register = first;
        for byte in &mut result {
            *byte = self.aux_read_register(register).await?;
            register = register.wrapping_add(1);
        }
        Ok(result)
    }

    /// Wake up the BMM150 on the AUX interface and read its factory trim
    /// values. Then start it at 30 Hz, and let the BMI270 read it in
    /// automatic mode.
    ///
    /// Returns the trim values, which are needed to decode the magnetometer
    /// frames.
    ///
    /// # Errors
    ///
    /// - [`Error::Bus`] when an I2C transaction fails.
    /// - [`Error::AuxBusy`] when a forwarded access does not finish.
    /// - [`Error::BmmChipId`] when the chip on the AUX bus is not a BMM150.
    ///
    /// After an error, the AUX interface can stay partly configured. Call
    /// [`Bmi270::disable_aux`] then.
    pub(super) async fn initialize_bmm150(&self) -> Result<Trim, Error> {
        // Prepare the AUX interface for manual access to the BMM150. The
        // `REG_PWR_CTRL` AUX bit is set only at the end, together with
        // automatic mode.
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

    /// Turn off the AUX interface after a failed BMM150 setup.
    ///
    /// An error here is only logged at debug level. The accelerometer and
    /// the gyroscope keep working in both cases. The next setup try repeats
    /// all steps.
    pub(super) async fn disable_aux(&self) {
        if self
            .write_register(REG_PWR_CTRL, PWR_CTRL_ACC_GYR)
            .await
            .is_err()
        {
            log::debug!("BMI270 auxiliary interface could not be disabled");
        }
    }

    /// Read the newest magnetometer frame, acceleration, rotation speed and
    /// timestamp in one I2C transaction.
    ///
    /// # Errors
    ///
    /// [`Error::Bus`] when the I2C transaction fails.
    pub(super) async fn read_sample(&self) -> Result<RawSample, Error> {
        // Read DATA_0 to DATA_19 (0x04 to 0x17) and then the three SENSORTIME
        // bytes (0x18 to 0x1A). Bosch documents that the chip copies
        // (shadows) the sensor time when a read starts in the data
        // registers. So the timestamp matches the data of this read, and it
        // needs no extra I2C transaction.
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
