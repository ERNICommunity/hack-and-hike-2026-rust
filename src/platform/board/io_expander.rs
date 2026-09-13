//! AW9523 board-control policy for CoreS3-Lite.
//!
//! The expander owns reset/enable lines for several onboard peripherals. Keep
//! those electrical details here; device drivers should only request the board
//! transition they need.

use esp_hal::delay::Delay;

const AW9523_ADDR: u8 = 0x58;

#[cfg(any(
    feature = "touch",
    feature = "speaker",
    all(feature = "display", feature = "touch")
))]
const PORT0_OUTPUT_REGISTER: u8 = 0x02;
#[cfg(any(feature = "display", feature = "camera"))]
const PORT1_OUTPUT_REGISTER: u8 = 0x03;
#[cfg(any(
    feature = "touch",
    feature = "speaker",
    all(feature = "display", feature = "touch")
))]
const PORT0_DIRECTION_REGISTER: u8 = 0x04;
#[cfg(any(feature = "display", feature = "camera"))]
const PORT1_DIRECTION_REGISTER: u8 = 0x05;
#[cfg(any(feature = "touch", feature = "speaker"))]
const GLOBAL_CONTROL_REGISTER: u8 = 0x11;
#[cfg(any(feature = "touch", feature = "speaker"))]
const PORT0_MODE_REGISTER: u8 = 0x12;
#[cfg(any(feature = "display", feature = "camera"))]
const PORT1_MODE_REGISTER: u8 = 0x13;

#[cfg(feature = "touch")]
const TOUCH_RESET: u8 = 1 << 0;
#[cfg(feature = "speaker")]
const SPEAKER_RESET: u8 = 1 << 2;
#[cfg(feature = "camera")]
const CAMERA_RESET: u8 = 1 << 0;
#[cfg(feature = "display")]
const LCD_RESET: u8 = 1 << 1;

// M5Stack's CoreS3 AW9523 bootstrap values. P0_2 normally appears high in the
// reference value (0x07); we deliberately hold it low here until the AW88298
// rail has been enabled and the speaker reset sequence is executed.
#[cfg(all(feature = "display", feature = "touch"))]
const PORT0_BOOT_OUTPUTS: u8 = 0b0000_0011;
#[cfg(all(feature = "display", feature = "touch"))]
const PORT1_BOOT_OUTPUTS: u8 = 0b1000_1111;
#[cfg(all(feature = "display", feature = "touch"))]
const PORT0_DIRECTIONS: u8 = 0b0001_1000;
#[cfg(all(feature = "display", feature = "touch"))]
const PORT1_DIRECTIONS: u8 = 0b0000_1100;
#[cfg(any(feature = "touch", feature = "speaker"))]
const PORT0_PUSH_PULL: u8 = 0b0001_0000;
#[cfg(all(feature = "display", feature = "touch"))]
const GPIO_MODE_ALL: u8 = 0xFF;

fn read_register<I2C>(i2c: &mut I2C, register: u8) -> Result<u8, I2C::Error>
where
    I2C: embedded_hal::i2c::I2c,
{
    let mut value = [0u8; 1];
    i2c.write_read(AW9523_ADDR, &[register], &mut value)?;
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
    i2c.write(AW9523_ADDR, &[register, next])
}

#[cfg(any(
    all(feature = "display", not(feature = "touch")),
    all(feature = "touch", not(feature = "display")),
))]
fn reset_pin(
    i2c: &mut impl embedded_hal::i2c::I2c,
    output_register: u8,
    direction_register: u8,
    mode_register: u8,
    reset_mask: u8,
    delay: &mut Delay,
) {
    // Make a single reset line self-contained without rewriting unrelated
    // expander pins owned by disabled capabilities.
    let _ = update_register_bits(i2c, mode_register, reset_mask, reset_mask);
    let _ = update_register_bits(i2c, output_register, reset_mask, 0);
    let _ = update_register_bits(i2c, direction_register, reset_mask, 0);
    delay.delay_millis(20u32);

    let _ = update_register_bits(i2c, output_register, reset_mask, reset_mask);
    delay.delay_millis(300u32);
}

/// Reset only the onboard LCD controller.
///
/// This path is used by display-only firmware so the disabled touch capability's
/// reset line is never manipulated.
#[cfg(all(feature = "display", not(feature = "touch")))]
pub(crate) fn reset_display(i2c: &mut impl embedded_hal::i2c::I2c, delay: &mut Delay) {
    reset_pin(
        i2c,
        PORT1_OUTPUT_REGISTER,
        PORT1_DIRECTION_REGISTER,
        PORT1_MODE_REGISTER,
        LCD_RESET,
        delay,
    );
}

/// Reset only the onboard touch controller.
///
/// This path is used by touch-only firmware so the disabled display capability's
/// reset line is never manipulated.
#[cfg(all(feature = "touch", not(feature = "display")))]
pub(crate) fn reset_touch(i2c: &mut impl embedded_hal::i2c::I2c, delay: &mut Delay) {
    // Match the board's push-pull policy for port 0 before driving TOUCH_RESET.
    let _ = i2c.write(AW9523_ADDR, &[GLOBAL_CONTROL_REGISTER, PORT0_PUSH_PULL]);
    reset_pin(
        i2c,
        PORT0_OUTPUT_REGISTER,
        PORT0_DIRECTION_REGISTER,
        PORT0_MODE_REGISTER,
        TOUCH_RESET,
        delay,
    );
}

/// Establish the CoreS3-Lite AW9523 GPIO policy and reset LCD + touch.
///
/// This runs once during bootstrap before CPU1 starts when both capabilities are
/// enabled. Keep the historical combined sequence for the full/default firmware
/// so this composition fix does not alter its board bring-up behavior. The
/// speaker reset line remains asserted.
#[cfg(all(feature = "display", feature = "touch"))]
pub(crate) fn reset_display_and_touch(i2c: &mut impl embedded_hal::i2c::I2c, delay: &mut Delay) {
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

/// Pulse the onboard GC0308 reset line on AW9523 P1_0.
#[cfg(feature = "camera")]
pub(crate) fn reset_camera<I2C>(i2c: &mut I2C, delay: &mut Delay) -> Result<(), I2C::Error>
where
    I2C: embedded_hal::i2c::I2c,
{
    // Make camera reset self-contained so camera-only firmware does not depend
    // on the display/touch bootstrap having configured AW9523 first. Drive the
    // latch low before switching P1_0 to an output to avoid a high-going glitch.
    update_register_bits(i2c, PORT1_MODE_REGISTER, CAMERA_RESET, CAMERA_RESET)?;
    update_register_bits(i2c, PORT1_OUTPUT_REGISTER, CAMERA_RESET, 0)?;
    update_register_bits(i2c, PORT1_DIRECTION_REGISTER, CAMERA_RESET, 0)?;
    delay.delay_millis(20u32);

    // GC0308 RESETB is active-low. Release it and allow the external 20 MHz
    // camera clock to run before SCCB access.
    update_register_bits(i2c, PORT1_OUTPUT_REGISTER, CAMERA_RESET, CAMERA_RESET)?;
    delay.delay_millis(20u32);
    Ok(())
}

/// Read the AW9523 registers relevant to the camera reset pin.
///
/// For P1_0 to release GC0308 RESETB, output bit 0 should be high, direction
/// bit 0 should be 0 (output), and mode bit 0 should be 1 (GPIO mode).
#[cfg(feature = "camera")]
pub(crate) fn camera_reset_registers<I2C>(i2c: &mut I2C) -> Result<(u8, u8, u8), I2C::Error>
where
    I2C: embedded_hal::i2c::I2c,
{
    Ok((
        read_register(i2c, PORT1_OUTPUT_REGISTER)?,
        read_register(i2c, PORT1_DIRECTION_REGISTER)?,
        read_register(i2c, PORT1_MODE_REGISTER)?,
    ))
}

/// Power and reset/release the onboard AW88298 amplifier.
///
/// CoreS3 speaker bring-up is a two-stage board operation: AXP2101 ALDO1 first
/// supplies the amplifier at 1.8 V, then AW9523 P0_2 is pulsed low -> high.
/// M5Stack's CoreS3 implementation holds reset low for 10 ms and waits 50 ms
/// after release before accessing AW88298 over I2C.
#[cfg(feature = "speaker")]
pub(crate) fn release_audio_amplifier<I2C>(
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

    update_register_bits(i2c, PORT0_OUTPUT_REGISTER, SPEAKER_RESET, SPEAKER_RESET)?;
    delay.delay_millis(50u32);
    Ok(())
}
