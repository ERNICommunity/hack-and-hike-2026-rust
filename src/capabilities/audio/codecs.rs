//! ES7210 microphone and AW88298 amplifier configuration.

use esp_hal::delay::Delay;

use crate::platform::{self, registers::Registers};

const ES7210_ADDR: u8 = 0x40;
const AW88298_ADDR: u8 = 0x36;

/// Power and configure both audio codecs for 16 kHz stereo 16-bit I2S.
pub(crate) fn init_codecs<I2C>(i2c: &mut I2C, delay: Delay) -> Result<(), I2C::Error>
where
    I2C: embedded_hal::i2c::I2c,
{
    init_es7210(i2c)?;
    init_aw88298(i2c, delay)
}

/// Configure ES7210 MIC1/MIC2 for stereo 16 kHz I2S input.
fn init_es7210<I2C>(i2c: &mut I2C) -> Result<(), I2C::Error>
where
    I2C: embedded_hal::i2c::I2c,
{
    // Reset, then the M5Unified register program for this board.
    const ES7210_INIT: &[(u8, u8)] = &[
        (0x00, 0xFF),
        (0x00, 0x41),
        (0x01, 0x1F),
        (0x06, 0x00),
        (0x07, 0x20),
        (0x08, 0x10),
        (0x09, 0x30),
        (0x0A, 0x30),
        (0x20, 0x0A),
        (0x21, 0x2A),
        (0x22, 0x0A),
        (0x23, 0x2A),
        (0x02, 0xC1),
        (0x04, 0x01),
        (0x05, 0x00),
        (0x11, 0x60),
        (0x40, 0x42),
        (0x41, 0x70),
        (0x42, 0x70),
        (0x43, 0x1B),
        (0x44, 0x1B),
        (0x45, 0x00),
        (0x46, 0x00),
        (0x47, 0x00),
        (0x48, 0x00),
        (0x49, 0x00),
        (0x4A, 0x00),
        (0x4B, 0x00),
        (0x4C, 0xFF),
        (0x01, 0x14),
    ];

    platform::power::enable_microphone(i2c)?;
    Registers::new(i2c, ES7210_ADDR).write_all(ES7210_INIT)
}

/// Release and configure the onboard AW88298 speaker amplifier for the same
/// 16 kHz, stereo, 16-bit I2S clock domain used by microphone capture.
fn init_aw88298<I2C>(i2c: &mut I2C, delay: Delay) -> Result<(), I2C::Error>
where
    I2C: embedded_hal::i2c::I2c,
{
    platform::io_expander::release_audio_amplifier(i2c, delay)?;

    // CoreS3 reference configuration from M5Unified. At 16 kHz the rate-table
    // index is 3, therefore register 0x06 is 0x14C3 for 16-bit stereo BCK mode.
    aw88298_write(i2c, 0x61, 0x0673)?; // boost disabled
    aw88298_write(i2c, 0x04, 0x4040)?; // I2S enabled, amplifier powered
    aw88298_write(i2c, 0x05, 0x0008)?; // unmuted
    aw88298_write(i2c, 0x06, 0x14C3)?; // 16 kHz, 16-bit x 2
    aw88298_write(i2c, 0x0C, 0x0064)?; // reference full-volume setting
    Ok(())
}

/// The AW88298 has 16-bit registers, unlike the other chips on the bus.
fn aw88298_write<I2C>(i2c: &mut I2C, register: u8, value: u16) -> Result<(), I2C::Error>
where
    I2C: embedded_hal::i2c::I2c,
{
    let [high, low] = value.to_be_bytes();
    i2c.write(AW88298_ADDR, &[register, high, low])
}
