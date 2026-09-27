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
//! `begin_frame` and `finish` wait for the sensor when they have to: at
//! the start, and after the ring buffer overflowed, for up to two frame
//! periods (half a second when the sensor sends nothing). A task on an interrupt executor must not wait: it would hold up
//! the task it interrupted, and the timer of both CPU cores. Such a task
//! uses [`Camera::service`] (empty the buffer, and start the capture again
//! when it stopped), [`Camera::advance`] (move on to the newest whole
//! frame) and [`Camera::current`] (the frame to use) instead. None of them
//! waits; an overflow then costs the frames on their way, and no time.
//!
//! The camera is optional, like the light and proximity sensor.
//! `Board::init` returns `None` for it when no sensor answers, and does not
//! panic.
//!
//! Automatic exposure and automatic white balance are on after start-up.
//! [`Camera::set_auto_adjust`] switches both off, which freezes the
//! brightness and the colours, and on again. A CPU1 task writes the sensor
//! registers, like the backlight task. Applications that never call it see
//! no change.

mod capture;
mod gc0308;
mod runtime;

use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use esp_hal::delay::Delay;
use log::{info, warn};

use crate::board::{i2c, io_expander, power};

pub(crate) use capture::Resources;
pub use capture::{Camera, CaptureStats, Frame, HEIGHT, WIDTH};
pub(crate) use runtime::spawn;

/// The state shared by the camera handle (CPU0) and the control task (CPU1).
struct Service {
    /// The newest auto adjust request that is not applied yet: `true` for
    /// on, `false` for locked. A new request replaces the old one.
    request: Signal<CriticalSectionRawMutex, bool>,
}

/// The one shared control state. A plain `static` is safe to use from both
/// cores, because the signal protects its value with a critical section.
static SERVICE: Service = Service {
    request: Signal::new(),
};

impl Camera {
    /// Switch automatic exposure and automatic white balance on (`true`) or
    /// off (`false`). Off freezes both: the sensor keeps the exposure and the
    /// colour gains it chose last, so the brightness no longer jumps. Both
    /// are on after start-up.
    ///
    /// Never waits: a task on CPU1 writes the sensor registers shortly
    /// after. When an earlier request is not applied yet, this one replaces
    /// it.
    pub fn set_auto_adjust(&mut self, enabled: bool) {
        SERVICE.request.signal(enabled);
    }
}

/// CPU1 side of the signal, used by the control task.
#[derive(Clone, Copy)]
pub(crate) struct Runtime {
    /// Points at the signal shared with the camera handle on CPU0.
    service: &'static Service,
}

impl Runtime {
    /// Wait for the next auto adjust request, and take it out of the signal.
    async fn next_request(self) -> bool {
        self.service.request.wait().await
    }
}

/// The CPU1 side of the auto adjust signal. The board creates it only when
/// the camera answered, and gives it to the control task.
pub(crate) fn runtime() -> Runtime {
    Runtime { service: &SERVICE }
}

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
/// [`Camera::begin_frame`] or [`Camera::service`].
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
