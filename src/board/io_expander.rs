//! AW9523 reset/enable policy for this board.

use esp_hal::delay::Delay;

const AW9523_ADDR: u8 = 0x58;

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
    i2c.write_read(AW9523_ADDR, &[register], &mut current)?;
    let next = (current[0] & !mask) | (value & mask);
    i2c.write(AW9523_ADDR, &[register, next])
}

/// Configure AW9523 directions and reset the LCD + touch controller.
///
/// This owns only the board wiring/reset sequence. LCD controller setup remains
/// in `screen`, and touch acquisition remains in `touch`.
pub fn reset_display_and_touch(i2c: &mut impl embedded_hal::i2c::I2c, delay: &mut Delay) {
    // Preserve the existing best-effort display/touch startup behavior.
    let _ = i2c.write(AW9523_ADDR, &[0x13, 0xFF]);

    // AW9523 direction registers: 0 = output, 1 = input.
    // P0_0 = FT6336 TOUCH_RST -> output.
    let _ = update_register_bits(i2c, 0x04, 1 << 0, 0);

    // P1_1 = LCD_RST -> output.
    // P1_2 = FT6336 TOUCH_INT -> input.
    let _ = update_register_bits(i2c, 0x05, (1 << 1) | (1 << 2), 1 << 2);

    // Reset LCD and touch controller together, preserving unrelated outputs.
    let _ = update_register_bits(i2c, 0x03, 1 << 1, 0);
    let _ = update_register_bits(i2c, 0x02, 1 << 0, 0);
    delay.delay_millis(20u32);

    let _ = update_register_bits(i2c, 0x03, 1 << 1, 1 << 1);
    let _ = update_register_bits(i2c, 0x02, 1 << 0, 1 << 0);

    delay.delay_millis(300u32);
}

/// Configure P0_2 as an output and release the onboard amplifier.
///
/// Amplifier device configuration remains the responsibility of the audio
/// service when/if it is added.
pub fn release_audio_amplifier<I2C>(
    i2c: &mut I2C,
    delay: &mut Delay,
) -> Result<(), I2C::Error>
where
    I2C: embedded_hal::i2c::I2c,
{
    update_register_bits(i2c, 0x04, 1 << 2, 0)?;
    update_register_bits(i2c, 0x02, 1 << 2, 1 << 2)?;
    delay.delay_millis(10u32);
    Ok(())
}
