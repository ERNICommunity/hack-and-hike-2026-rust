//! The camera.
//!
//! [`Board::init`](crate::Board::init) sets up the GC0308 sensor once, over
//! the shared I2C bus. After that, frames arrive through the `LCD_CAM`
//! peripheral of the ESP32-S3 and DMA (direct memory access), on CPU0.
//! Frames are 320x240 pixels in RGB565 (16-bit colour), most significant
//! byte first. The display uses the same format, so rows go to the screen
//! without conversion.
//!
//! ```ignore
//! if let Some(camera) = camera.as_mut()
//!     && let Some(mut frame) = camera.begin_frame()
//! {
//!     display.surface(SCREEN).render_from(&mut frame);
//!     frame.finish();
//! }
//! ```
//!
//! The first `begin_frame` after start-up or after [`Camera::pause`] waits
//! for a complete frame. Later calls return the newest complete frame at
//! once. While the display sends a frame, the camera copies the next frame
//! from its small DMA ring buffer. `finish` then switches to the frame that
//! was completed in the meantime, or waits for the next one.
//!
//! The ring buffer holds only a few milliseconds of data, and the sensor
//! never pauses. So a loop that does other work between frames must call
//! [`Camera::pump`] there, and it should not sleep. The private `capture`
//! module explains the buffers.
//!
//! The camera is optional, like the light and proximity sensor.
//! `Board::init` returns `None` for it when no sensor answers, and does not
//! panic.

mod capture;
mod gc0308;

use esp_hal::delay::Delay;
use log::{info, warn};

use crate::board::{i2c, io_expander, power};

pub(crate) use capture::Resources;
pub use capture::{Camera, Frame, HEIGHT, WIDTH};

/// Milliseconds to wait after the camera power rails turn on, before the
/// reset pulse. The voltages need this time to become stable.
const RAIL_SETTLE_MS: u32 = 10;

/// Why the camera start-up (bring-up) failed.
#[derive(Debug)]
enum BringUpError<E> {
    /// An I2C transfer failed: to the power chip (PMIC, power management IC),
    /// the IO expander or the sensor. Usually the chip did not answer.
    Bus(E),
    /// A sensor answered, but its product ID (PID) is not the GC0308 ID.
    UnexpectedPid(u8),
}

/// Power the sensor, program its registers and set up the capture pipeline.
///
/// Return `None` and log a warning when an I2C transfer fails or the sensor
/// is not a GC0308. Capturing starts later, with the first
/// [`Camera::begin_frame`].
///
/// The sensor's control bus uses the same pins as the board's system I2C
/// bus, but at 100 kHz instead of 400 kHz. So this function borrows the bus
/// resources twice: first at 400 kHz for the power chip and the IO expander,
/// then at 100 kHz for the sensor. After that, the resources are free again
/// for the runtime bus.
pub(crate) fn bring_up(
    bus: &mut i2c::Resources<'static>,
    delay: Delay,
    resources: Resources,
) -> Option<Camera> {
    let powered = {
        let mut system_i2c = i2c::init(bus.reborrow());
        power::enable_camera(&mut system_i2c)
            .and_then(|()| {
                delay.delay_millis(RAIL_SETTLE_MS);
                io_expander::reset_camera(&mut system_i2c, delay)
            })
            .map_err(BringUpError::Bus)
    };
    let programmed = powered.and_then(|()| {
        let mut sccb = i2c::init_camera_sccb(bus.reborrow());
        gc0308::init(&mut sccb, delay)
    });

    match programmed {
        Ok(()) => {
            info!("GC0308 camera ready");
            Some(capture::init(resources))
        }
        Err(BringUpError::Bus(error)) => {
            warn!("Camera disabled: I2C error {:?}", error);
            None
        }
        Err(BringUpError::UnexpectedPid(pid)) => {
            warn!("Camera disabled: unexpected sensor ID 0x{:02x}", pid);
            None
        }
    }
}
