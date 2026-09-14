//! The camera.
//!
//! The GC0308 sensor is programmed once during bring-up over the shared I2C
//! bus; afterwards frames stream through the ESP32-S3's `LCD_CAM` peripheral
//! and DMA on CPU0. Frames are 320x240 RGB565 with the most significant byte
//! first, the same format the display takes, so rows go straight to the
//! panel without conversion.
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
//! The camera is the one part of the board that may be missing: bring-up
//! returns `None` instead of panicking when no sensor answers.

mod capture;
mod gc0308;

use esp_hal::delay::Delay;
use log::{info, warn};

use crate::platform::{i2c, io_expander, power};

pub(crate) use capture::Resources;
pub use capture::{Camera, Frame, HEIGHT, WIDTH};

/// Settle time between enabling the camera power rails and pulsing reset.
const RAIL_SETTLE_MS: u32 = 10;

/// Why the camera could not be brought up.
#[derive(Debug)]
enum BringUpError<E> {
    /// The PMIC, IO expander or sensor did not answer on I2C.
    Bus(E),
    /// A sensor answered, but it is not a GC0308.
    UnexpectedPid(u8),
}

/// Power the sensor, program it and start the capture pipeline.
///
/// The sensor's control bus is the board's system I2C bus at 100 kHz instead
/// of 400 kHz, so this borrows the bus resources twice (power at full speed,
/// then the sensor at the slower speed) and releases them again for the
/// runtime bus.
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
