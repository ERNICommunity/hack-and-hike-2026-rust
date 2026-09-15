//! Reset lines behind the AW9523 IO expander.
//!
//! The ESP32-S3 has too few pins for everything on the board. So the reset
//! lines of the LCD, the touch controller, the camera and the speaker
//! amplifier are connected to this IO expander. The ESP32-S3 controls the
//! expander over I2C. The expander has two 8-bit ports. Each port has an
//! output, a direction and a mode register.
//!
//! Each function here does one complete step of the board start-up, for
//! example "reset the LCD and the touch controller". Drivers never change
//! expander bits directly.

use esp_hal::delay::Delay;

use super::registers::Registers;

/// I2C address of the AW9523.
const AW9523_ADDR: u8 = 0x58;

/// Output latch of port 0: the level each output pin drives, one bit per pin.
const PORT0_OUTPUT_REGISTER: u8 = 0x02;
/// Output latch of port 1: the level each output pin drives, one bit per pin.
const PORT1_OUTPUT_REGISTER: u8 = 0x03;
/// Direction of the port 0 pins: a 0 bit makes that pin an output, a 1 bit an input.
const PORT0_DIRECTION_REGISTER: u8 = 0x04;
/// Direction of the port 1 pins: a 0 bit makes that pin an output, a 1 bit an input.
const PORT1_DIRECTION_REGISTER: u8 = 0x05;
/// Global control register. Among other settings, it selects push-pull or
/// open-drain outputs for port 0 (see [`PORT0_PUSH_PULL`]).
const GLOBAL_CONTROL_REGISTER: u8 = 0x11;
/// Mode of the port 0 pins: a 1 bit makes that pin a normal GPIO, a 0 bit an
/// LED current driver (the chip can also dim LEDs directly).
const PORT0_MODE_REGISTER: u8 = 0x12;
/// Mode of the port 1 pins: a 1 bit makes that pin a normal GPIO, a 0 bit an
/// LED current driver.
const PORT1_MODE_REGISTER: u8 = 0x13;

// The reset lines. All are active low: 0 holds the chip in reset, 1 lets it
// run.
/// Touch controller reset, port 0 bit 0.
const TOUCH_RESET: u8 = 1 << 0;
/// Speaker amplifier reset, port 0 bit 2.
const SPEAKER_RESET: u8 = 1 << 2;
/// Camera sensor reset, port 1 bit 0.
const CAMERA_RESET: u8 = 1 << 0;
/// LCD controller reset, port 1 bit 1.
const LCD_RESET: u8 = 1 << 1;

/// How long the LCD and touch reset lines are held low, in milliseconds.
const LCD_TOUCH_RESET_PULSE_MS: u32 = 20;
/// Wait after the release of the LCD and touch resets, in milliseconds. Both
/// chips need this time to start before the drivers talk to them.
const LCD_TOUCH_RESET_SETTLE_MS: u32 = 300;
/// How long the camera reset line is held low, in milliseconds.
const CAMERA_RESET_PULSE_MS: u32 = 20;
/// Wait after the release of the camera reset, in milliseconds. The sensor
/// needs a running clock before the first SCCB (camera I2C) transfer.
const CAMERA_CLOCK_SETTLE_MS: u32 = 20;
/// Wait after the amplifier's 1.8 V rail is switched on, in milliseconds,
/// before the reset line changes.
const AMPLIFIER_RAIL_SETTLE_MS: u32 = 10;
/// How long the amplifier reset line is held low, in milliseconds.
const AMPLIFIER_RESET_PULSE_MS: u32 = 10;
/// Wait after the release of the amplifier reset, in milliseconds, before
/// its registers are used over I2C.
const AMPLIFIER_SETTLE_MS: u32 = 50;

// The start values of M5Stack's CoreS3 code for the AW9523. One difference:
// P0_2 (port 0, bit 2) is high in the M5Stack value (0x07). Here it stays low
// until the AW88298 amplifier rail is on and the speaker reset sequence runs.
/// Port 0 output levels at boot: bits 0 and 1 high (bit 0 lets the touch
/// controller run), bit 2 low (holds the speaker amplifier in reset).
const PORT0_BOOT_OUTPUTS: u8 = 0b0000_0011;
/// Port 1 output levels at boot: bits 0-3 and 7 high. Bit 0 lets the camera
/// run, and bit 1 lets the LCD run.
const PORT1_BOOT_OUTPUTS: u8 = 0b1000_1111;
/// Port 0 directions at boot: bits 3 and 4 are inputs, all other pins outputs.
const PORT0_DIRECTIONS: u8 = 0b0001_1000;
/// Port 1 directions at boot: bits 2 and 3 are inputs, all other pins outputs.
const PORT1_DIRECTIONS: u8 = 0b0000_1100;
/// Global control value for push-pull outputs on port 0. A push-pull pin
/// drives both the high and the low level. An open-drain pin only pulls low.
const PORT0_PUSH_PULL: u8 = 0b0001_0000;
/// Mode value that puts every pin of a port in GPIO mode.
const GPIO_MODE_ALL: u8 = 0xFF;

/// The AW9523's registers on `i2c`.
fn expander<I2C: embedded_hal::i2c::I2c>(i2c: &mut I2C) -> Registers<'_, I2C> {
    Registers::new(i2c, AW9523_ADDR)
}

/// Set the start values of all AW9523 pins, then reset the LCD and the touch
/// controller.
///
/// Board bring-up runs this once, before CPU1 starts. The speaker amplifier
/// stays in reset until
/// [`release_audio_amplifier`] enables its rail and releases it.
///
/// Takes about 320 ms: a 20 ms reset pulse, then 300 ms for the chips to
/// start.
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

/// Reset the GC0308 camera sensor with a pulse on AW9523 pin P1_0 (port 1,
/// bit 0).
///
/// Call it after the camera power rails are on. Takes about 40 ms.
pub(crate) fn reset_camera<I2C>(i2c: &mut I2C, delay: Delay) -> Result<(), I2C::Error>
where
    I2C: embedded_hal::i2c::I2c,
{
    let mut expander = expander(i2c);
    // Make P1_0 a GPIO, and set its output latch low before it becomes an
    // output. So the pin never drives a short high pulse.
    expander.update_bits(PORT1_MODE_REGISTER, CAMERA_RESET, CAMERA_RESET)?;
    expander.update_bits(PORT1_OUTPUT_REGISTER, CAMERA_RESET, 0)?;
    expander.update_bits(PORT1_DIRECTION_REGISTER, CAMERA_RESET, 0)?;
    delay.delay_millis(CAMERA_RESET_PULSE_MS);

    // The GC0308 reset input (RESETB) is active low. Release it. Then wait,
    // so the 20 MHz camera clock from the board runs before the first SCCB
    // transfer.
    expander.update_bits(PORT1_OUTPUT_REGISTER, CAMERA_RESET, CAMERA_RESET)?;
    delay.delay_millis(CAMERA_CLOCK_SETTLE_MS);
    Ok(())
}

/// Power the AW88298 speaker amplifier, reset it and let it run.
///
/// The speaker bring-up has two steps. First, the AXP2101 power chip enables
/// the amplifier rail (ALDO1) at 1.8 V. Then AW9523 pin P0_2 goes low and
/// back to high. The timing is the same as in M5Stack's CoreS3 code: 10 ms
/// for the rail, 10 ms in reset, and 50 ms after the release before the
/// first I2C access to the AW88298.
pub(crate) fn release_audio_amplifier<I2C>(i2c: &mut I2C, delay: Delay) -> Result<(), I2C::Error>
where
    I2C: embedded_hal::i2c::I2c,
{
    super::power::enable_speaker_amplifier(i2c)?;
    delay.delay_millis(AMPLIFIER_RAIL_SETTLE_MS);

    // Make sure that the reset pin is a push-pull GPIO, even if the expander
    // has other settings from earlier firmware. Set the output latch low
    // before the pin becomes an output, so the pin never drives a short high
    // pulse.
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
