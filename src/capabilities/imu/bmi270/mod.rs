//! BMI270 transport, initialization, auxiliary sensor hub, and sample decoding.

mod config;

use embassy_time::{Duration, Timer};

use crate::platform::i2c::SystemI2cBus;

use super::bmm150;

const BMI270_ADDR: u8 = 0x69;
const BMI270_CHIP_ID: u8 = 0x24;

const REG_CHIP_ID: u8 = 0x00;
const REG_STATUS: u8 = 0x03;
const REG_AUX_X_LSB: u8 = 0x04;
const REG_INTERNAL_STATUS: u8 = 0x21;
const REG_ACC_CONF: u8 = 0x40;
const REG_ACC_RANGE: u8 = 0x41;
const REG_GYR_CONF: u8 = 0x42;
const REG_GYR_RANGE: u8 = 0x43;
const REG_AUX_CONF: u8 = 0x44;
const REG_AUX_DEV_ID: u8 = 0x4B;
const REG_AUX_IF_CONF: u8 = 0x4C;
const REG_AUX_RD_ADDR: u8 = 0x4D;
const REG_AUX_WR_ADDR: u8 = 0x4E;
const REG_AUX_WR_DATA: u8 = 0x4F;
const REG_INIT_CTRL: u8 = 0x59;
const REG_INIT_ADDR_0: u8 = 0x5B;
const REG_INIT_DATA: u8 = 0x5E;
const REG_AUX_IF_TRIM: u8 = 0x68;
const REG_IF_CONF: u8 = 0x6B;
const REG_PWR_CONF: u8 = 0x7C;
const REG_PWR_CTRL: u8 = 0x7D;
const REG_CMD: u8 = 0x7E;

const CMD_SOFT_RESET: u8 = 0xB6;
const CONFIG_LOAD_OK: u8 = 0x01;
const AUX_BUSY: u8 = 1 << 2;

// Acceleration stays at 100 Hz. Gyro runs at 400 Hz, performance filtering,
// normal bandwidth: lower group delay and much higher bandwidth during fast yaw
// without increasing host I2C traffic. Range remains +/-2000 dps.
const ACC_CONF_100HZ: u8 = 0xA8;
const GYR_CONF_400HZ: u8 = 0xAA;
const ACC_RANGE_4G: u8 = 0x01;
const GYR_RANGE_2000DPS: u8 = 0x00;
const PWR_CTRL_ACC_GYR: u8 = 0x06;
const PWR_CTRL_ACC_GYR_AUX: u8 = 0x0F;

// Poll the 30 Hz BMM150 at 100 Hz inside BMI270. This does not add host I2C
// traffic, but halves worst-case AUX pickup latency compared with 50 Hz.
const AUX_CONF_100HZ: u8 = 0x48;
const AUX_IF_DATA_MODE_8_BYTES: u8 = 0x4F;
const AUX_IF_MANUAL_MODE: u8 = 0x80;
const AUX_IF_TRIM_2K_PULLUP: u8 = 0x03;

const ACC_G_PER_LSB: f32 = 4.0 / 32768.0;
const GYR_DPS_PER_LSB: f32 = 2000.0 / 32768.0;
const SENSOR_STARTUP: Duration = Duration::from_millis(50);

/// BMI270 gyroscope data registers run faster internally to reduce phase lag
/// during quick turns while the host keeps the proven 100 Hz I2C cadence.
pub(super) const GYRO_SENSOR_ODR_HZ: u32 = 400;

#[derive(Clone, Copy, Debug)]
pub(super) enum Error {
    Bus,
    ChipId(u8),
    ConfigStatus(u8),
    AuxBusy,
    BmmChipId(u8),
}

#[derive(Clone, Copy)]
pub(super) struct RawSample {
    pub(super) accel_g: [f32; 3],
    pub(super) gyro_dps: [f32; 3],
    pub(super) mag_data: [u8; 8],
    pub(super) sensor_time: u32,
}

pub(super) struct Bmi270 {
    bus: SystemI2cBus,
}

impl Bmi270 {
    pub(super) const fn new(bus: SystemI2cBus) -> Self {
        Self { bus }
    }

    async fn write_register(&self, register: u8, value: u8) -> Result<(), Error> {
        let mut i2c = self.bus.lock().await;
        i2c.write_async(BMI270_ADDR, &[register, value])
            .await
            .map_err(|_| Error::Bus)
    }

    async fn read_register(&self, register: u8) -> Result<u8, Error> {
        let mut value = [0u8; 1];
        let mut i2c = self.bus.lock().await;
        i2c.write_read_async(BMI270_ADDR, &[register], &mut value)
            .await
            .map_err(|_| Error::Bus)?;
        Ok(value[0])
    }

    async fn upload_config(&self) -> Result<(), Error> {
        for (offset, chunk) in config::MAXIMUM_FIFO_CONFIG.chunks(32).enumerate() {
            let byte_offset = offset * 32;
            let word_address = byte_offset >> 1;
            let address = [
                REG_INIT_ADDR_0,
                (word_address & 0x0F) as u8,
                (word_address >> 4) as u8,
            ];

            let mut packet = [0u8; 33];
            packet[0] = REG_INIT_DATA;
            packet[1..1 + chunk.len()].copy_from_slice(chunk);

            let mut i2c = self.bus.lock().await;
            i2c.write_async(BMI270_ADDR, &address)
                .await
                .map_err(|_| Error::Bus)?;
            i2c.write_async(BMI270_ADDR, &packet[..1 + chunk.len()])
                .await
                .map_err(|_| Error::Bus)?;
        }

        Ok(())
    }

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

    async fn wait_aux_idle(&self) -> Result<(), Error> {
        for _ in 0..8 {
            if self.read_register(REG_STATUS).await? & AUX_BUSY == 0 {
                return Ok(());
            }
            Timer::after(Duration::from_millis(1)).await;
        }
        Err(Error::AuxBusy)
    }

    async fn aux_write_register(&self, register: u8, value: u8) -> Result<(), Error> {
        self.write_register(REG_AUX_WR_DATA, value).await?;
        self.write_register(REG_AUX_WR_ADDR, register).await?;
        self.wait_aux_idle().await
    }

    async fn aux_read_register(&self, register: u8) -> Result<u8, Error> {
        self.write_register(REG_AUX_IF_CONF, AUX_IF_MANUAL_MODE)
            .await?;
        self.write_register(REG_AUX_RD_ADDR, register).await?;
        self.wait_aux_idle().await?;
        self.read_register(REG_AUX_X_LSB).await
    }

    async fn aux_read_array<const N: usize>(&self, first: u8) -> Result<[u8; N], Error> {
        let mut result = [0u8; N];
        let mut register = first;
        for byte in &mut result {
            *byte = self.aux_read_register(register).await?;
            register = register.wrapping_add(1);
        }
        Ok(result)
    }

    pub(super) async fn initialize_bmm150(&self) -> Result<bmm150::Trim, Error> {
        self.write_register(REG_IF_CONF, 0x20).await?;
        self.write_register(REG_PWR_CONF, 0x00).await?;
        self.write_register(REG_PWR_CTRL, PWR_CTRL_ACC_GYR).await?;
        self.write_register(REG_AUX_IF_TRIM, AUX_IF_TRIM_2K_PULLUP)
            .await?;
        self.write_register(REG_AUX_IF_CONF, AUX_IF_MANUAL_MODE)
            .await?;
        self.write_register(REG_AUX_DEV_ID, bmm150::ADDRESS << 1)
            .await?;

        self.aux_write_register(bmm150::REG_POWER_CONTROL, bmm150::SOFT_RESET_AND_POWER)
            .await?;
        Timer::after(Duration::from_millis(5)).await;

        let chip_id = self.aux_read_register(bmm150::REG_CHIP_ID).await?;
        if chip_id != bmm150::CHIP_ID {
            return Err(Error::BmmChipId(chip_id));
        }

        let x1_y1 = self.aux_read_array::<2>(bmm150::DIG_X1).await?;
        let z4_x2_y2 = self.aux_read_array::<4>(bmm150::DIG_Z4_LSB).await?;
        let z2_to_xy1 = self.aux_read_array::<10>(bmm150::DIG_Z2_LSB).await?;
        let trim = bmm150::Trim::from_registers(x1_y1, z4_x2_y2, z2_to_xy1);

        self.aux_write_register(bmm150::REG_REP_XY, bmm150::REP_XY_REGULAR)
            .await?;
        self.aux_write_register(bmm150::REG_REP_Z, bmm150::REP_Z_REGULAR)
            .await?;
        self.aux_write_register(bmm150::REG_OP_MODE, bmm150::NORMAL_30HZ)
            .await?;

        self.write_register(REG_AUX_CONF, AUX_CONF_100HZ).await?;
        self.write_register(REG_AUX_IF_CONF, AUX_IF_DATA_MODE_8_BYTES)
            .await?;
        self.write_register(REG_AUX_RD_ADDR, bmm150::REG_DATA_X_LSB)
            .await?;
        self.write_register(REG_PWR_CTRL, PWR_CTRL_ACC_GYR_AUX)
            .await?;
        Timer::after(Duration::from_millis(10)).await;

        Ok(trim)
    }

    pub(super) async fn disable_aux(&self) {
        let _ = self.write_register(REG_PWR_CTRL, PWR_CTRL_ACC_GYR).await;
    }

    pub(super) async fn read_sample(&self) -> Result<RawSample, Error> {
        // DATA_0..DATA_19 followed by the three SENSORTIME bytes. Bosch defines
        // sensor time as shadowed at the start of a burst that begins in the data
        // registers, so this gives a coherent 25.6 kHz timestamp at no extra I2C
        // transaction cost.
        let mut bytes = [0u8; 23];
        let mut i2c = self.bus.lock().await;
        i2c.write_read_async(BMI270_ADDR, &[REG_AUX_X_LSB], &mut bytes)
            .await
            .map_err(|_| Error::Bus)?;
        drop(i2c);

        let mut mag_data = [0u8; 8];
        mag_data.copy_from_slice(&bytes[..8]);

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
