//! Register setup of the two audio chips (codecs): the ES7210 microphone ADC
//! and the AW88298 speaker amplifier.
//!
//! A codec here is a chip that converts between sound and digital samples.
//! The ES7210 is an ADC (analog-to-digital converter): it turns the signals
//! of the two microphones into samples. The AW88298 is a digital amplifier:
//! it turns samples into a signal for the loudspeaker.
//!
//! Both chips are configured once, over I2C, for the same I2S (Inter-IC
//! Sound) format: 16 kHz, stereo, 16-bit samples. The register values come
//! from M5Stack's M5Unified library for this board.

use esp_hal::delay::Delay;

use crate::board::{self, registers::Registers};

/// 7-bit I2C address of the ES7210 microphone ADC.
const ES7210_ADDR: u8 = 0x40;
/// 7-bit I2C address of the AW88298 speaker amplifier.
const AW88298_ADDR: u8 = 0x36;

/// Power and configure both audio codecs for 16 kHz, stereo, 16-bit I2S.
///
/// # Errors
///
/// The error of the first I2C transfer that fails.
pub(crate) fn init_codecs<I2C>(i2c: &mut I2C, delay: Delay) -> Result<(), I2C::Error>
where
    I2C: embedded_hal::i2c::I2c,
{
    init_es7210(i2c)?;
    init_aw88298(i2c, delay)
}

/// Turn on the microphone power rail, and configure the ES7210 inputs MIC1
/// and MIC2 for stereo 16 kHz I2S.
///
/// # Errors
///
/// The error of the first I2C transfer that fails.
fn init_es7210<I2C>(i2c: &mut I2C) -> Result<(), I2C::Error>
where
    I2C: embedded_hal::i2c::I2c,
{
    // The first two writes reset the chip. The other writes are the
    // M5Unified register program for this board.
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

    board::power::enable_microphone(i2c)?;
    Registers::new(i2c, ES7210_ADDR).write_all(ES7210_INIT)
}

/// Power the AW88298 speaker amplifier, release its reset, and configure it.
///
/// It uses the same I2S format and the same clocks as the microphone
/// capture: 16 kHz, stereo, 16-bit samples.
///
/// # Errors
///
/// The error of the first I2C transfer that fails.
fn init_aw88298<I2C>(i2c: &mut I2C, delay: Delay) -> Result<(), I2C::Error>
where
    I2C: embedded_hal::i2c::I2c,
{
    board::io_expander::release_audio_amplifier(i2c, delay)?;

    // The CoreS3 configuration from M5Unified. M5Unified finds the sample
    // rate in a table. For 16 kHz, the table index is 3. So register 0x06 is
    // 0x14C3: sample rate index 3, and the bit clock (BCK) mode for two
    // 16-bit samples per frame.
    aw88298_write(i2c, 0x61, 0x0673)?; // boost converter off
    aw88298_write(i2c, 0x04, 0x4040)?; // I2S on, amplifier powered
    aw88298_write(i2c, 0x05, 0x0008)?; // not muted
    aw88298_write(i2c, 0x06, 0x14C3)?; // 16 kHz, 2 x 16-bit samples
    aw88298_write(i2c, 0x0C, 0x0064)?; // volume: the M5Unified full-volume value
    Ok(())
}

/// Write one register of the AW88298. Its registers have 16 bits, unlike the
/// 8-bit registers of the other chips on the bus. The value is sent most
/// significant byte first.
///
/// # Errors
///
/// The I2C error when the transfer fails.
fn aw88298_write<I2C>(i2c: &mut I2C, register: u8, value: u16) -> Result<(), I2C::Error>
where
    I2C: embedded_hal::i2c::I2c,
{
    let [high, low] = value.to_be_bytes();
    i2c.write(AW88298_ADDR, &[register, high, low])
}
