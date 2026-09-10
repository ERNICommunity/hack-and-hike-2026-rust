//! AXP2101 power-rail policy for this board.

const AXP2101_ADDR: u8 = 0x34;
const OUTPUT_ENABLE_REGISTER: u8 = 0x90;

const ALDO1_VOLTAGE_REGISTER: u8 = 0x92;
const ALDO1_ENABLE: u8 = 1 << 0;
const ALDO2_VOLTAGE_REGISTER: u8 = 0x93;
const ALDO2_ENABLE: u8 = 1 << 1;
const ALDO3_VOLTAGE_REGISTER: u8 = 0x94;
const ALDO3_ENABLE: u8 = 1 << 2;
const BLDO1_ENABLE: u8 = 1 << 4;
const BLDO2_ENABLE: u8 = 1 << 5;
const CAMERA_POWER_ENABLE: u8 = ALDO3_ENABLE | BLDO1_ENABLE | BLDO2_ENABLE;
const DLDO1_VOLTAGE_REGISTER: u8 = 0x99;
const DLDO1_ENABLE: u8 = 1 << 7;

// AXP2101 ALDO voltage encoding is Vout/100mV - 5 in this range.
const SPEAKER_ALDO1_1V8_CODE: u8 = 18 - 5;
const MICROPHONE_ALDO2_3V3_CODE: u8 = 33 - 5;
const CAMERA_ALDO3_3V3_CODE: u8 = 33 - 5;

// The CoreS3 backlight is powered from DLDO1. Keep runtime dimming inside the
// documented 2.6-3.3 V operating range rather than exposing PMIC register codes
// to presentation code. AXP2101 encodes this range as 0x15..=0x1C.
const LCD_BACKLIGHT_MIN_CODE: u8 = 0x15;
const LCD_BACKLIGHT_MAX_CODE: u8 = 0x1C;

fn read_register<I2C>(i2c: &mut I2C, register: u8) -> Result<u8, I2C::Error>
where
    I2C: embedded_hal::i2c::I2c,
{
    let mut value = [0u8; 1];
    i2c.write_read(AXP2101_ADDR, &[register], &mut value)?;
    Ok(value[0])
}

fn update_register_bits<I2C>(
    i2c: &mut I2C,
    register: u8,
    mask: u8,
    value: u8,
) -> Result<(), I2C::Error>
where
    I2C: embedded_hal::i2c::I2c,
{
    let current = read_register(i2c, register)?;
    let next = (current & !mask) | (value & mask);
    i2c.write(AXP2101_ADDR, &[register, next])
}

async fn update_register_bits_async<I2C>(
    i2c: &mut I2C,
    register: u8,
    mask: u8,
    value: u8,
) -> Result<(), I2C::Error>
where
    I2C: embedded_hal_async::i2c::I2c,
{
    let mut current = [0u8; 1];
    i2c.write_read(AXP2101_ADDR, &[register], &mut current)
        .await?;
    let next = (current[0] & !mask) | (value & mask);
    i2c.write(AXP2101_ADDR, &[register, next]).await
}

/// Enable the LCD backlight rail (DLDO1) at 3.3 V.
///
/// Display-controller configuration itself remains owned by `display`.
pub(crate) fn enable_lcd_backlight(i2c: &mut impl embedded_hal::i2c::I2c) {
    // Preserve the existing best-effort startup behavior for the display rail.
    let _ = i2c.write(
        AXP2101_ADDR,
        &[DLDO1_VOLTAGE_REGISTER, LCD_BACKLIGHT_MAX_CODE],
    );
    let _ = update_register_bits(i2c, OUTPUT_ENABLE_REGISTER, DLDO1_ENABLE, DLDO1_ENABLE);
}

/// Apply a semantic 1-100% LCD brightness request to the CoreS3 backlight rail.
///
/// The hardware has eight effective voltage steps in its supported backlight
/// range. Runtime dimming never disables DLDO1: 1% maps to 2.6 V and 100% to
/// 3.3 V. Turning display power off is deliberately not a slider operation.
pub(crate) async fn set_lcd_backlight<I2C>(i2c: &mut I2C, percent: u8) -> Result<(), I2C::Error>
where
    I2C: embedded_hal_async::i2c::I2c,
{
    debug_assert!((1..=100).contains(&percent));

    let span = u16::from(LCD_BACKLIGHT_MAX_CODE - LCD_BACKLIGHT_MIN_CODE);
    let scaled = (u16::from(percent - 1) * span + 49) / 99;
    let code = LCD_BACKLIGHT_MIN_CODE + scaled as u8;

    i2c.write(AXP2101_ADDR, &[DLDO1_VOLTAGE_REGISTER, code])
        .await?;
    update_register_bits_async(i2c, OUTPUT_ENABLE_REGISTER, DLDO1_ENABLE, DLDO1_ENABLE).await
}

/// Enable the microphone rail (ALDO2) at 3.3 V.
///
/// ES7210 register configuration itself remains owned by `audio`.
pub(crate) fn enable_microphone<I2C>(i2c: &mut I2C) -> Result<(), I2C::Error>
where
    I2C: embedded_hal::i2c::I2c,
{
    i2c.write(
        AXP2101_ADDR,
        &[ALDO2_VOLTAGE_REGISTER, MICROPHONE_ALDO2_3V3_CODE],
    )?;
    update_register_bits(i2c, OUTPUT_ENABLE_REGISTER, ALDO2_ENABLE, ALDO2_ENABLE)
}

/// Enable the onboard GC0308 camera power domain.
///
/// CoreS3/CoreS3-Lite camera bring-up requires AXP2101 ALDO3 plus BLDO1 and
/// BLDO2 to be enabled together. Espressif's CoreS3 BSP uses the same 0x34 mask
/// in register 0x90 for BSP_FEATURE_CAMERA, while ALDO3 register 0x94 is set to
/// 3.3 V. The AW9523 owns the separate camera reset line.
pub(crate) fn enable_camera<I2C>(i2c: &mut I2C) -> Result<(), I2C::Error>
where
    I2C: embedded_hal::i2c::I2c,
{
    i2c.write(
        AXP2101_ADDR,
        &[ALDO3_VOLTAGE_REGISTER, CAMERA_ALDO3_3V3_CODE],
    )?;
    update_register_bits(
        i2c,
        OUTPUT_ENABLE_REGISTER,
        CAMERA_POWER_ENABLE,
        CAMERA_POWER_ENABLE,
    )
}

/// Read the raw AXP2101 registers that prove the camera power configuration.
///
/// Expected register 0x90 bits are ALDO3 + BLDO1 + BLDO2 (mask 0x34), with
/// voltage code 0x1c in register 0x94 (3.3 V). On the normal CoreS3 bootstrap
/// state this typically changes 0x8f to 0xbf. This does not measure the physical
/// rails, but it distinguishes PMIC-programming failures from downstream faults.
pub(crate) fn camera_power_registers<I2C>(i2c: &mut I2C) -> Result<(u8, u8), I2C::Error>
where
    I2C: embedded_hal::i2c::I2c,
{
    Ok((
        read_register(i2c, OUTPUT_ENABLE_REGISTER)?,
        read_register(i2c, ALDO3_VOLTAGE_REGISTER)?,
    ))
}

/// Enable the onboard AW88298 supply rail (ALDO1) at 1.8 V.
///
/// This is the PMIC half of CoreS3 speaker bring-up. AW9523 owns the separate
/// speaker-enable gate, and `audio` owns the AW88298 device registers.
pub(crate) fn enable_speaker_amplifier<I2C>(i2c: &mut I2C) -> Result<(), I2C::Error>
where
    I2C: embedded_hal::i2c::I2c,
{
    i2c.write(
        AXP2101_ADDR,
        &[ALDO1_VOLTAGE_REGISTER, SPEAKER_ALDO1_1V8_CODE],
    )?;
    update_register_bits(i2c, OUTPUT_ENABLE_REGISTER, ALDO1_ENABLE, ALDO1_ENABLE)
}
