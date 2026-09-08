//! AXP2101 power-rail policy for this board.

const AXP2101_ADDR: u8 = 0x34;
const DLDO1_VOLTAGE_REGISTER: u8 = 0x99;
const OUTPUT_ENABLE_REGISTER: u8 = 0x90;
const DLDO1_ENABLE: u8 = 1 << 7;

// The CoreS3 backlight is powered from DLDO1. Keep runtime dimming inside the
// documented 2.6-3.3 V operating range rather than exposing PMIC register codes
// to presentation code. AXP2101 encodes this range as 0x15..=0x1C.
const LCD_BACKLIGHT_MIN_CODE: u8 = 0x15;
const LCD_BACKLIGHT_MAX_CODE: u8 = 0x1C;

fn update_register_bits<I2C>(
    i2c: &mut I2C,
    register: u8,
    mask: u8,
    value: u8,
) -> Result<(), I2C::Error>
where
    I2C: embedded_hal::i2c::I2c,
{
    let mut current = [0u8; 1];
    i2c.write_read(AXP2101_ADDR, &[register], &mut current)?;
    let next = (current[0] & !mask) | (value & mask);
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
pub fn enable_lcd_backlight(i2c: &mut impl embedded_hal::i2c::I2c) {
    // Preserve the existing best-effort startup behavior for the display rail.
    let _ = i2c.write(AXP2101_ADDR, &[DLDO1_VOLTAGE_REGISTER, LCD_BACKLIGHT_MAX_CODE]);
    let _ = update_register_bits(
        i2c,
        OUTPUT_ENABLE_REGISTER,
        DLDO1_ENABLE,
        DLDO1_ENABLE,
    );
}

/// Apply a semantic 1-100% LCD brightness request to the CoreS3 backlight rail.
///
/// The hardware has eight effective voltage steps in its supported backlight
/// range. Runtime dimming never disables DLDO1: 1% maps to 2.6 V and 100% to
/// 3.3 V. Turning display power off is deliberately not a slider operation.
pub async fn set_lcd_backlight<I2C>(i2c: &mut I2C, percent: u8) -> Result<(), I2C::Error>
where
    I2C: embedded_hal_async::i2c::I2c,
{
    debug_assert!((1..=100).contains(&percent));

    let span = u16::from(LCD_BACKLIGHT_MAX_CODE - LCD_BACKLIGHT_MIN_CODE);
    let scaled = (u16::from(percent - 1) * span + 49) / 99;
    let code = LCD_BACKLIGHT_MIN_CODE + scaled as u8;

    i2c.write(AXP2101_ADDR, &[DLDO1_VOLTAGE_REGISTER, code])
        .await?;
    update_register_bits_async(
        i2c,
        OUTPUT_ENABLE_REGISTER,
        DLDO1_ENABLE,
        DLDO1_ENABLE,
    )
    .await
}

/// Enable the microphone rail (ALDO2) at 3.3 V.
///
/// ES7210 register configuration itself remains owned by `audio`.
pub fn enable_microphone<I2C>(i2c: &mut I2C) -> Result<(), I2C::Error>
where
    I2C: embedded_hal::i2c::I2c,
{
    i2c.write(AXP2101_ADDR, &[0x93, 0x1C])?;
    update_register_bits(i2c, OUTPUT_ENABLE_REGISTER, 1 << 1, 1 << 1)
}
