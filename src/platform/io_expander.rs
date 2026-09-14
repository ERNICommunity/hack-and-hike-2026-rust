//! Reset lines behind the AW9523 IO expander.
//!
//! The ESP32-S3 has too few pins for everything on the board, so the reset
//! lines of the LCD, the touch controller, the camera and the speaker
//! amplifier hang off this I2C GPIO expander instead. It has two 8-bit ports,
//! each with an output, a direction and a mode register.
//!
//! The functions here perform one board transition each (reset these chips,
//! release that one); drivers never touch expander bits directly.

use esp_hal::delay::Delay;

use super::registers::Registers;

/// I2C address of the AW9523.
const AW9523_ADDR: u8 = 0x58;

const PORT0_OUTPUT_REGISTER: u8 = 0x02;
const PORT1_OUTPUT_REGISTER: u8 = 0x03;
const PORT0_DIRECTION_REGISTER: u8 = 0x04;
const PORT1_DIRECTION_REGISTER: u8 = 0x05;
const GLOBAL_CONTROL_REGISTER: u8 = 0x11;
const PORT0_MODE_REGISTER: u8 = 0x12;
const PORT1_MODE_REGISTER: u8 = 0x13;

// Reset lines, all active low: 0 holds the chip in reset.
/// Touch controller reset, port 0 bit 0.
const TOUCH_RESET: u8 = 1 << 0;
/// Speaker amplifier reset, port 0 bit 2.
const SPEAKER_RESET: u8 = 1 << 2;
/// Camera sensor reset, port 1 bit 0.
const CAMERA_RESET: u8 = 1 << 0;
/// LCD controller reset, port 1 bit 1.
const LCD_RESET: u8 = 1 << 1;

const LCD_TOUCH_RESET_PULSE_MS: u32 = 20;
const LCD_TOUCH_RESET_SETTLE_MS: u32 = 300;
const CAMERA_RESET_PULSE_MS: u32 = 20;
const CAMERA_CLOCK_SETTLE_MS: u32 = 20;
const AMPLIFIER_RAIL_SETTLE_MS: u32 = 10;
const AMPLIFIER_RESET_PULSE_MS: u32 = 10;
const AMPLIFIER_SETTLE_MS: u32 = 50;

// M5Stack's CoreS3 AW9523 bootstrap values. P0_2 normally appears high in the
// reference value (0x07); it is held low here until the AW88298 rail has been
// enabled and the speaker reset sequence is executed.
const PORT0_BOOT_OUTPUTS: u8 = 0b0000_0011;
const PORT1_BOOT_OUTPUTS: u8 = 0b1000_1111;
const PORT0_DIRECTIONS: u8 = 0b0001_1000;
const PORT1_DIRECTIONS: u8 = 0b0000_1100;
const PORT0_PUSH_PULL: u8 = 0b0001_0000;
const GPIO_MODE_ALL: u8 = 0xFF;

/// The AW9523's registers on `i2c`.
fn expander<I2C: embedded_hal::i2c::I2c>(i2c: &mut I2C) -> Registers<'_, I2C> {
    Registers::new(i2c, AW9523_ADDR)
}

/// Establish the CoreS3-Lite AW9523 GPIO policy and reset LCD + touch.
///
/// This runs once during board bring-up before CPU1 starts. The speaker reset
/// line stays asserted until the amplifier rail is enabled.
pub(crate) fn reset_display_and_touch<I2C>(i2c: &mut I2C, delay: Delay) -> Result<(), I2C::Error>
where
    I2C: embedded_hal::i2c::I2c,
{
    let mut expander = expander(i2c);
    expander.write_all(&[
        (PORT0_OUTPUT_REGISTER, PORT0_BOOT_OUTPUTS),
        (PORT1_OUTPUT_REGISTER, PORT1_BOOT_OUTPUTS),
        (PORT0_DIRECTION_REGISTER, PORT0_DIRECTIONS),
        (PORT1_DIRECTION_REGISTER, PORT1_DIRECTIONS),
        (GLOBAL_CONTROL_REGISTER, PORT0_PUSH_PULL),
        (PORT0_MODE_REGISTER, GPIO_MODE_ALL),
        (PORT1_MODE_REGISTER, GPIO_MODE_ALL),
    ])?;

    expander.update_bits(PORT1_OUTPUT_REGISTER, LCD_RESET, 0)?;
    expander.update_bits(PORT0_OUTPUT_REGISTER, TOUCH_RESET, 0)?;
    delay.delay_millis(LCD_TOUCH_RESET_PULSE_MS);

    expander.update_bits(PORT1_OUTPUT_REGISTER, LCD_RESET, LCD_RESET)?;
    expander.update_bits(PORT0_OUTPUT_REGISTER, TOUCH_RESET, TOUCH_RESET)?;
    delay.delay_millis(LCD_TOUCH_RESET_SETTLE_MS);
    Ok(())
}

/// Pulse the onboard GC0308 reset line on AW9523 P1_0.
pub(crate) fn reset_camera<I2C>(i2c: &mut I2C, delay: Delay) -> Result<(), I2C::Error>
where
    I2C: embedded_hal::i2c::I2c,
{
    let mut expander = expander(i2c);
    // Drive the latch low before switching P1_0 to an output to avoid a
    // high-going glitch.
    expander.update_bits(PORT1_MODE_REGISTER, CAMERA_RESET, CAMERA_RESET)?;
    expander.update_bits(PORT1_OUTPUT_REGISTER, CAMERA_RESET, 0)?;
    expander.update_bits(PORT1_DIRECTION_REGISTER, CAMERA_RESET, 0)?;
    delay.delay_millis(CAMERA_RESET_PULSE_MS);

    // GC0308 RESETB is active-low. Release it and allow the external 20 MHz
    // camera clock to run before SCCB access.
    expander.update_bits(PORT1_OUTPUT_REGISTER, CAMERA_RESET, CAMERA_RESET)?;
    delay.delay_millis(CAMERA_CLOCK_SETTLE_MS);
    Ok(())
}

/// Power and reset/release the onboard AW88298 amplifier.
///
/// CoreS3 speaker bring-up is a two-stage board operation: AXP2101 ALDO1 first
/// supplies the amplifier at 1.8 V, then AW9523 P0_2 is pulsed low -> high.
/// M5Stack's CoreS3 implementation holds reset low for 10 ms and waits 50 ms
/// after release before accessing AW88298 over I2C.
pub(crate) fn release_audio_amplifier<I2C>(i2c: &mut I2C, delay: Delay) -> Result<(), I2C::Error>
where
    I2C: embedded_hal::i2c::I2c,
{
    super::power::enable_speaker_amplifier(i2c)?;
    delay.delay_millis(AMPLIFIER_RAIL_SETTLE_MS);

    // Make the control pin unambiguously a push-pull GPIO before resetting the
    // amplifier. Drive the output latch low before changing its direction to
    // avoid a high-going glitch if firmware inherited a different expander state.
    let mut expander = expander(i2c);
    expander.write(GLOBAL_CONTROL_REGISTER, PORT0_PUSH_PULL)?;
    expander.update_bits(PORT0_MODE_REGISTER, SPEAKER_RESET, SPEAKER_RESET)?;
    expander.update_bits(PORT0_OUTPUT_REGISTER, SPEAKER_RESET, 0)?;
    expander.update_bits(PORT0_DIRECTION_REGISTER, SPEAKER_RESET, 0)?;
    delay.delay_millis(AMPLIFIER_RESET_PULSE_MS);

    expander.update_bits(PORT0_OUTPUT_REGISTER, SPEAKER_RESET, SPEAKER_RESET)?;
    delay.delay_millis(AMPLIFIER_SETTLE_MS);
    Ok(())
}
