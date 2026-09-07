//! AXP2101 power-rail policy for this board.

const AXP2101_ADDR: u8 = 0x34;

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

/// Enable the LCD backlight rail (DLDO1) at 3.3 V.
///
/// Display-controller configuration itself remains owned by `display`.
pub fn enable_lcd_backlight(i2c: &mut impl embedded_hal::i2c::I2c) {
    // Preserve the existing best-effort startup behavior for the display rail.
    let _ = i2c.write(AXP2101_ADDR, &[0x99, 0x1C]);
    let _ = update_register_bits(i2c, 0x90, 1 << 7, 1 << 7);
}

/// Enable the microphone rail (ALDO2) at 3.3 V.
///
/// ES7210 register configuration itself remains owned by `audio`.
pub fn enable_microphone<I2C>(i2c: &mut I2C) -> Result<(), I2C::Error>
where
    I2C: embedded_hal::i2c::I2c,
{
    i2c.write(AXP2101_ADDR, &[0x93, 0x1C])?;
    update_register_bits(i2c, 0x90, 1 << 1, 1 << 1)
}
