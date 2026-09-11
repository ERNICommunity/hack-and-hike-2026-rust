//! CoreS3 Lite camera capability.
//!
//! GC0308 sensor programming is a startup-only control-plane concern, while the
//! LCD_CAM/DMA frame pipeline is CPU0-owned data-plane capture. Both remain
//! concrete and private behind this small capability facade.

mod capture;
mod gc0308;

use esp_hal::delay::Delay;

pub(crate) use capture::{Camera, Frame, HEIGHT, Resources, WIDTH, init};

/// Byte ordering and pixel encoding exposed by camera frames.
///
/// The GC0308 is configured to emit RGB565 with the most-significant byte first.
/// Applications can therefore pass scanline bytes directly to an RGB565-BE
/// display path without conversion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PixelFormat {
    Rgb565Be,
}

pub(crate) const PIXEL_FORMAT: PixelFormat = PixelFormat::Rgb565Be;

impl Frame<'_> {
    pub(crate) const fn pixel_format(&self) -> PixelFormat {
        PIXEL_FORMAT
    }
}

/// Program the GC0308 over a startup-only hardware SCCB/I2C owner.
///
/// GPIO12/GPIO11 are shared with the board's normal system I2C bus. Bootstrap
/// deliberately drops its temporary board-I2C driver, creates a fresh 100 kHz
/// hardware owner for this call, then drops it again before constructing the
/// persistent 400 kHz runtime bus. This mirrors M5Stack's CoreS3 camera bring-up.
pub(crate) fn init_sensor<I2C>(i2c: &mut I2C, delay: &mut Delay) -> Result<u8, I2C::Error>
where
    I2C: embedded_hal::i2c::I2c,
{
    gc0308::init(i2c, delay)
}

pub(crate) const EXPECTED_SENSOR_PID: u8 = gc0308::EXPECTED_PID;
