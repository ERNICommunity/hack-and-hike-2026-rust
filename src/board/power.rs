//! Power rails of the AXP2101 power management chip.
//!
//! The AXP2101 has several low-dropout regulators (LDOs), each with a voltage
//! register and an enable bit. On the CoreS3 Lite they supply:
//!
//! | Rail | Voltage | Supplies |
//! | --- | --- | --- |
//! | ALDO1 | 1.8 V | speaker amplifier |
//! | ALDO2 | 3.3 V | microphones |
//! | ALDO3, BLDO1, BLDO2 | 3.3 V | camera |
//! | DLDO1 | 2.6-3.3 V | LCD backlight (its voltage sets the brightness) |
//!
//! Everything else on the board is powered before the firmware starts.

use super::registers::{AsyncRegisters, Registers};

/// I2C address of the AXP2101.
const AXP2101_ADDR: u8 = 0x34;
/// One enable bit per LDO rail.
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

// AXP2101 ALDO voltage encoding is Vout/100mV - 5 in this range.
/// ALDO1 voltage code for 1.8 V.
const SPEAKER_ALDO1_1V8_CODE: u8 = 18 - 5;
/// ALDO2 voltage code for 3.3 V.
const MICROPHONE_ALDO2_3V3_CODE: u8 = 33 - 5;
/// ALDO3 voltage code for 3.3 V.
const CAMERA_ALDO3_3V3_CODE: u8 = 33 - 5;

// The CoreS3 backlight is powered from DLDO1. Runtime dimming stays inside
// the documented 2.6-3.3 V operating range, which AXP2101 encodes as
// 0x15..=0x1C: eight usable steps.
/// DLDO1 voltage code for 2.6 V, the dimmest backlight setting.
const LCD_BACKLIGHT_MIN_CODE: u8 = 0x15;
/// DLDO1 voltage code for 3.3 V, the brightest backlight setting.
const LCD_BACKLIGHT_MAX_CODE: u8 = 0x1C;

/// The AXP2101's registers on `i2c`.
fn pmic<I2C: embedded_hal::i2c::I2c>(i2c: &mut I2C) -> Registers<'_, I2C> {
    Registers::new(i2c, AXP2101_ADDR)
}

/// Enable the LCD backlight rail (DLDO1) at full brightness.
pub(crate) fn enable_lcd_backlight<I2C>(i2c: &mut I2C) -> Result<(), I2C::Error>
where
    I2C: embedded_hal::i2c::I2c,
{
    let mut pmic = pmic(i2c);
    pmic.write(DLDO1_VOLTAGE_REGISTER, LCD_BACKLIGHT_MAX_CODE)?;
    pmic.update_bits(OUTPUT_ENABLE_REGISTER, DLDO1_ENABLE, DLDO1_ENABLE)
}

/// The DLDO1 voltage code for a 1-100 % brightness: 1 % is 2.6 V, 100 % is
/// 3.3 V. Dimming never switches the rail off.
fn backlight_code(percent: u8) -> u8 {
    const PERCENT_SPAN: u16 = 99;
    let steps = LCD_BACKLIGHT_MAX_CODE - LCD_BACKLIGHT_MIN_CODE;
    let offset = u16::from(percent.clamp(1, 100) - 1);
    let scaled = (offset * u16::from(steps) + PERCENT_SPAN / 2) / PERCENT_SPAN;
    LCD_BACKLIGHT_MIN_CODE + scaled as u8
}

/// Apply a 1-100 % backlight brightness to the DLDO1 rail.
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
/// ES7210 register configuration itself remains owned by `audio`.
pub(crate) fn enable_microphone<I2C>(i2c: &mut I2C) -> Result<(), I2C::Error>
where
    I2C: embedded_hal::i2c::I2c,
{
    let mut pmic = pmic(i2c);
    pmic.write(ALDO2_VOLTAGE_REGISTER, MICROPHONE_ALDO2_3V3_CODE)?;
    pmic.update_bits(OUTPUT_ENABLE_REGISTER, ALDO2_ENABLE, ALDO2_ENABLE)
}

/// Enable the onboard GC0308 camera power domain.
///
/// CoreS3/CoreS3-Lite camera bring-up requires AXP2101 ALDO3 plus BLDO1 and
/// BLDO2 to be enabled together, with ALDO3 at 3.3 V; Espressif's CoreS3 BSP
/// uses the same mask. The AW9523 owns the separate camera reset line.
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

/// Enable the onboard AW88298 supply rail (ALDO1) at 1.8 V.
///
/// This is the PMIC half of CoreS3 speaker bring-up. AW9523 owns the separate
/// speaker-enable gate, and `audio` owns the AW88298 device registers.
pub(crate) fn enable_speaker_amplifier<I2C>(i2c: &mut I2C) -> Result<(), I2C::Error>
where
    I2C: embedded_hal::i2c::I2c,
{
    let mut pmic = pmic(i2c);
    pmic.write(ALDO1_VOLTAGE_REGISTER, SPEAKER_ALDO1_1V8_CODE)?;
    pmic.update_bits(OUTPUT_ENABLE_REGISTER, ALDO1_ENABLE, ALDO1_ENABLE)
}
