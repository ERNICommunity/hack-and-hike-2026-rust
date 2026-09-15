//! Power rails of the AXP2101 power management chip (PMIC).
//!
//! The AXP2101 has several low-dropout regulators (LDOs). Each LDO makes a
//! supply voltage for other chips. This output is called a power rail. Each
//! LDO has a voltage register and an enable bit. On the CoreS3 Lite they
//! supply:
//!
//! | Rail | Voltage | Supplies |
//! | --- | --- | --- |
//! | ALDO1 | 1.8 V | speaker amplifier |
//! | ALDO2 | 3.3 V | microphones |
//! | ALDO3 | 3.3 V | camera |
//! | BLDO1, BLDO2 | chip default | camera |
//! | DLDO1 | 2.6-3.3 V | LCD backlight (its voltage sets the brightness) |
//!
//! Everything else on the board is powered before the firmware starts.

use super::registers::{AsyncRegisters, Registers};

/// I2C address of the AXP2101.
const AXP2101_ADDR: u8 = 0x34;
/// Output enable register: one enable bit for each LDO rail.
const OUTPUT_ENABLE_REGISTER: u8 = 0x90;

/// Output voltage of ALDO1, the speaker amplifier rail.
const ALDO1_VOLTAGE_REGISTER: u8 = 0x92;
/// Enable bit of ALDO1 in [`OUTPUT_ENABLE_REGISTER`].
const ALDO1_ENABLE: u8 = 1 << 0;
/// Output voltage of ALDO2, the microphone rail.
const ALDO2_VOLTAGE_REGISTER: u8 = 0x93;
/// Enable bit of ALDO2 in [`OUTPUT_ENABLE_REGISTER`].
const ALDO2_ENABLE: u8 = 1 << 1;
/// Output voltage of ALDO3, one of the three camera rails.
const ALDO3_VOLTAGE_REGISTER: u8 = 0x94;
/// Enable bit of ALDO3 in [`OUTPUT_ENABLE_REGISTER`].
const ALDO3_ENABLE: u8 = 1 << 2;
/// Enable bit of BLDO1, a camera rail, in [`OUTPUT_ENABLE_REGISTER`]. Its
/// voltage is left at the chip's default.
const BLDO1_ENABLE: u8 = 1 << 4;
/// Enable bit of BLDO2, a camera rail, in [`OUTPUT_ENABLE_REGISTER`]. Its
/// voltage is left at the chip's default.
const BLDO2_ENABLE: u8 = 1 << 5;
/// All three camera rails, switched on together.
const CAMERA_POWER_ENABLE: u8 = ALDO3_ENABLE | BLDO1_ENABLE | BLDO2_ENABLE;
/// Output voltage of DLDO1, the LCD backlight rail.
const DLDO1_VOLTAGE_REGISTER: u8 = 0x99;
/// Enable bit of DLDO1 in [`OUTPUT_ENABLE_REGISTER`].
const DLDO1_ENABLE: u8 = 1 << 7;

// The AXP2101 encodes an ALDO voltage as (voltage / 100 mV) - 5. Code 0 is
// 0.5 V, and each step adds 100 mV.
/// ALDO1 voltage code for 1.8 V.
const SPEAKER_ALDO1_1V8_CODE: u8 = 18 - 5;
/// ALDO2 voltage code for 3.3 V.
const MICROPHONE_ALDO2_3V3_CODE: u8 = 33 - 5;
/// ALDO3 voltage code for 3.3 V.
const CAMERA_ALDO3_3V3_CODE: u8 = 33 - 5;

// DLDO1 powers the CoreS3 backlight. DLDO1 uses the same encoding as the
// ALDOs. Dimming stays inside the documented operating range of 2.6-3.3 V.
// The AXP2101 codes for this range are 0x15..=0x1C: eight brightness levels.
/// DLDO1 voltage code for 2.6 V, the dimmest backlight setting.
const LCD_BACKLIGHT_MIN_CODE: u8 = 0x15;
/// DLDO1 voltage code for 3.3 V, the brightest backlight setting.
const LCD_BACKLIGHT_MAX_CODE: u8 = 0x1C;

/// The AXP2101's registers on `i2c`.
fn pmic<I2C: embedded_hal::i2c::I2c>(i2c: &mut I2C) -> Registers<'_, I2C> {
    Registers::new(i2c, AXP2101_ADDR)
}

/// Enable the LCD backlight rail (DLDO1) at full brightness (3.3 V).
pub(crate) fn enable_lcd_backlight<I2C>(i2c: &mut I2C) -> Result<(), I2C::Error>
where
    I2C: embedded_hal::i2c::I2c,
{
    let mut pmic = pmic(i2c);
    pmic.write(DLDO1_VOLTAGE_REGISTER, LCD_BACKLIGHT_MAX_CODE)?;
    pmic.update_bits(OUTPUT_ENABLE_REGISTER, DLDO1_ENABLE, DLDO1_ENABLE)
}

/// The DLDO1 voltage code for a brightness of 1-100 %: 1 % is 2.6 V, 100 %
/// is 3.3 V. The percentage is rounded to the nearest of the eight codes.
/// Values outside 1-100 are clamped. Dimming never switches the rail off.
fn backlight_code(percent: u8) -> u8 {
    const PERCENT_SPAN: u16 = 99;
    let steps = LCD_BACKLIGHT_MAX_CODE - LCD_BACKLIGHT_MIN_CODE;
    let offset = u16::from(percent.clamp(1, 100) - 1);
    let scaled = (offset * u16::from(steps) + PERCENT_SPAN / 2) / PERCENT_SPAN;
    LCD_BACKLIGHT_MIN_CODE + scaled as u8
}

/// Set the DLDO1 rail to a backlight brightness of 1-100 %, and make sure
/// the rail is enabled.
pub(crate) async fn set_lcd_backlight<I2C>(i2c: &mut I2C, percent: u8) -> Result<(), I2C::Error>
where
    I2C: embedded_hal_async::i2c::I2c,
{
    let mut pmic = AsyncRegisters::new(i2c, AXP2101_ADDR);
    pmic.write(DLDO1_VOLTAGE_REGISTER, backlight_code(percent))
        .await?;
    pmic.update_bits(OUTPUT_ENABLE_REGISTER, DLDO1_ENABLE, DLDO1_ENABLE)
        .await
}

/// Enable the microphone rail (ALDO2) at 3.3 V.
///
/// The registers of the ES7210 (the microphone ADC, analog-to-digital
/// converter) are set up in `audio`.
pub(crate) fn enable_microphone<I2C>(i2c: &mut I2C) -> Result<(), I2C::Error>
where
    I2C: embedded_hal::i2c::I2c,
{
    let mut pmic = pmic(i2c);
    pmic.write(ALDO2_VOLTAGE_REGISTER, MICROPHONE_ALDO2_3V3_CODE)?;
    pmic.update_bits(OUTPUT_ENABLE_REGISTER, ALDO2_ENABLE, ALDO2_ENABLE)
}

/// Enable the three power rails of the GC0308 camera sensor.
///
/// The camera needs ALDO3, BLDO1 and BLDO2 enabled together, with ALDO3 at
/// 3.3 V. Espressif's CoreS3 BSP (board support package) uses the same bits.
/// The camera reset line is on the AW9523 IO expander; see
/// [`io_expander::reset_camera`](super::io_expander::reset_camera).
pub(crate) fn enable_camera<I2C>(i2c: &mut I2C) -> Result<(), I2C::Error>
where
    I2C: embedded_hal::i2c::I2c,
{
    let mut pmic = pmic(i2c);
    pmic.write(ALDO3_VOLTAGE_REGISTER, CAMERA_ALDO3_3V3_CODE)?;
    pmic.update_bits(
        OUTPUT_ENABLE_REGISTER,
        CAMERA_POWER_ENABLE,
        CAMERA_POWER_ENABLE,
    )
}

/// Enable the rail of the AW88298 speaker amplifier (ALDO1) at 1.8 V.
///
/// This is the first half of the speaker bring-up. The second half is the
/// amplifier reset line on the AW9523 IO expander; see
/// [`io_expander::release_audio_amplifier`](super::io_expander::release_audio_amplifier).
/// The AW88298 registers are set up in `audio`.
pub(crate) fn enable_speaker_amplifier<I2C>(i2c: &mut I2C) -> Result<(), I2C::Error>
where
    I2C: embedded_hal::i2c::I2c,
{
    let mut pmic = pmic(i2c);
    pmic.write(ALDO1_VOLTAGE_REGISTER, SPEAKER_ALDO1_1V8_CODE)?;
    pmic.update_bits(OUTPUT_ENABLE_REGISTER, ALDO1_ENABLE, ALDO1_ENABLE)
}
