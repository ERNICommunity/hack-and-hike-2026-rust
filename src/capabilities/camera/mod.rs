//! CoreS3 Lite camera capability.
//!
//! GC0308 sensor programming is a startup-only control-plane concern, while the
//! LCD_CAM/DMA frame pipeline is CPU0-owned data-plane capture. Both remain
//! concrete and private behind this small capability facade.
//!
//! Frames are RGB565 with the most-significant byte first, so scanline bytes can
//! be passed straight to the display's raw RGB565 path.

mod capture;
mod gc0308;

use esp_hal::delay::Delay;

use crate::platform::{io_expander, power};

pub use capture::{Camera, Frame, HEIGHT, WIDTH};
pub(crate) use capture::{Resources, init};

/// Settle time between enabling the camera power rail and releasing reset.
const RAIL_SETTLE_MS: u32 = 10;

/// Why the camera could not be brought up. The rest of the board keeps working.
#[derive(Debug)]
pub(crate) enum BringUpError<E> {
    /// The PMIC, IO expander or sensor did not answer on I2C.
    Bus(E),
    /// A sensor answered, but it is not a GC0308.
    UnexpectedPid(u8),
}

/// Enable the camera power domain and pulse the sensor reset line.
pub(crate) fn power_on<I2C>(i2c: &mut I2C, delay: Delay) -> Result<(), I2C::Error>
where
    I2C: embedded_hal::i2c::I2c,
{
    power::enable_camera(i2c)?;
    delay.delay_millis(RAIL_SETTLE_MS);
    io_expander::reset_camera(i2c, delay)
}

/// Program the GC0308 over a startup-only 100 kHz SCCB/I2C owner.
///
/// GPIO12/GPIO11 are shared with the board's normal system I2C bus. Board
/// bring-up drops its temporary I2C driver, creates a fresh 100 kHz owner for
/// this call, then drops it again before constructing the persistent 400 kHz
/// runtime bus. This mirrors M5Stack's CoreS3 camera bring-up.
pub(crate) fn init_sensor<I2C>(i2c: &mut I2C, delay: Delay) -> Result<(), BringUpError<I2C::Error>>
where
    I2C: embedded_hal::i2c::I2c,
{
    let pid = gc0308::init(i2c, delay).map_err(BringUpError::Bus)?;
    if pid == gc0308::EXPECTED_PID {
        Ok(())
    } else {
        Err(BringUpError::UnexpectedPid(pid))
    }
}
