//! AW9523 board-control policy for CoreS3-Lite.
//!
//! The expander owns reset/enable lines for several onboard peripherals. Keep
//! those electrical details here; device drivers should only request the board
//! transition they need.

use esp_hal::delay::Delay;

const AW9523_ADDR: u8 = 0x58;

const PORT0_OUTPUT_REGISTER: u8 = 0x02;
const PORT1_OUTPUT_REGISTER: u8 = 0x03;
const PORT0_DIRECTION_REGISTER: u8 = 0x04;
const PORT1_DIRECTION_REGISTER: u8 = 0x05;
const GLOBAL_CONTROL_REGISTER: u8 = 0x11;
const PORT0_MODE_REGISTER: u8 = 0x12;
const PORT1_MODE_REGISTER: u8 = 0x13;

const TOUCH_RESET: u8 = 1 << 0;
const SPEAKER_RESET: u8 = 1 << 2;
const LCD_RESET: u8 = 1 << 1;

// M5Stack's CoreS3 AW9523 bootstrap values. P0_2 normally appears high in the
// reference value (0x07); we deliberately hold it low here until the AW88298
// rail has been enabled and the speaker reset sequence is executed.
const PORT0_BOOT_OUTPUTS: u8 = 0b0000_0011;
const PORT1_BOOT_OUTPUTS: u8 = 0b1000_1111;
const PORT0_DIRECTIONS: u8 = 0b0001_1000;
const PORT1_DIRECTIONS: u8 = 0b0000_1100;
const PORT0_PUSH_PULL: u8 = 0b0001_0000;
const GPIO_MODE_ALL: u8 = 0xFF;

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

/// Establish the CoreS3-Lite AW9523 GPIO policy and reset LCD + touch.
///
/// This runs once during bootstrap before CPU1 starts. The display/touch path
/// historically treated expander setup as best-effort, so keep that behavior
/// while putting the expander into the same GPIO/push-pull mode used by the
/// board reference implementation. The speaker reset line remains asserted.
pub fn reset_display_and_touch(i2c: &mut impl embedded_hal::i2c::I2c, delay: &mut Delay) {
    let _ = i2c.write(AW9523_ADDR, &[PORT0_OUTPUT_REGISTER, PORT0_BOOT_OUTPUTS]);
    let _ = i2c.write(AW9523_ADDR, &[PORT1_OUTPUT_REGISTER, PORT1_BOOT_OUTPUTS]);
    let _ = i2c.write(AW9523_ADDR, &[PORT0_DIRECTION_REGISTER, PORT0_DIRECTIONS]);
    let _ = i2c.write(AW9523_ADDR, &[PORT1_DIRECTION_REGISTER, PORT1_DIRECTIONS]);
    let _ = i2c.write(AW9523_ADDR, &[GLOBAL_CONTROL_REGISTER, PORT0_PUSH_PULL]);
    let _ = i2c.write(AW9523_ADDR, &[PORT0_MODE_REGISTER, GPIO_MODE_ALL]);
    let _ = i2c.write(AW9523_ADDR, &[PORT1_MODE_REGISTER, GPIO_MODE_ALL]);

    // Reset LCD and touch together while leaving AW88298 held in reset.
    let _ = update_register_bits(i2c, PORT1_OUTPUT_REGISTER, LCD_RESET, 0);
    let _ = update_register_bits(i2c, PORT0_OUTPUT_REGISTER, TOUCH_RESET, 0);
    delay.delay_millis(20u32);

    let _ = update_register_bits(i2c, PORT1_OUTPUT_REGISTER, LCD_RESET, LCD_RESET);
    let _ = update_register_bits(i2c, PORT0_OUTPUT_REGISTER, TOUCH_RESET, TOUCH_RESET);
    delay.delay_millis(300u32);
}

/// Power and reset/release the onboard AW88298 amplifier.
///
/// CoreS3 speaker bring-up is a two-stage board operation: AXP2101 ALDO1 first
/// supplies the amplifier at 1.8 V, then AW9523 P0_2 is pulsed low -> high.
/// M5Stack's CoreS3 implementation holds reset low for 10 ms and waits 50 ms
/// after release before accessing AW88298 over I2C.
pub fn release_audio_amplifier<I2C>(
    i2c: &mut I2C,
    delay: &mut Delay,
) -> Result<(), I2C::Error>
where
    I2C: embedded_hal::i2c::I2c,
{
    super::power::enable_speaker_amplifier(i2c)?;
    delay.delay_millis(10u32);

    // Make the control pin unambiguously a push-pull GPIO before resetting the
    // amplifier. Drive the output latch low before changing its direction to
    // avoid a high-going glitch if firmware inherited a different expander state.
    i2c.write(AW9523_ADDR, &[GLOBAL_CONTROL_REGISTER, PORT0_PUSH_PULL])?;
    update_register_bits(i2c, PORT0_MODE_REGISTER, SPEAKER_RESET, SPEAKER_RESET)?;
    update_register_bits(i2c, PORT0_OUTPUT_REGISTER, SPEAKER_RESET, 0)?;
    update_register_bits(i2c, PORT0_DIRECTION_REGISTER, SPEAKER_RESET, 0)?;
    delay.delay_millis(10u32);

    update_register_bits(
        i2c,
        PORT0_OUTPUT_REGISTER,
        SPEAKER_RESET,
        SPEAKER_RESET,
    )?;
    delay.delay_millis(50u32);
    Ok(())
}
