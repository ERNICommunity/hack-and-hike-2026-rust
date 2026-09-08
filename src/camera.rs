//! CPU0-owned CoreS3 Lite camera capture.
//!
//! The onboard GC0308 emits native QVGA RGB565 over its 8-bit DVP bus. LCD_CAM
//! receives frames into two permanently allocated, cache-line aligned PSRAM
//! buffers. One buffer can be rendered while DMA fills the other, keeping each
//! capture aligned to a camera VSYNC boundary.

mod gc0308;

use esp_hal::{
    delay::Delay,
    dma::{DmaError, DmaRxBuf, ExternalBurstConfig},
    lcd_cam::{
        LcdCam,
        cam::{Camera as CameraDriver, CameraTransfer, Config as CameraConfig},
    },
    peripherals::{
        DMA_CH2, GPIO15, GPIO16, GPIO38, GPIO39, GPIO40, GPIO41, GPIO42, GPIO45, GPIO46,
        GPIO47, GPIO48, LCD_CAM,
    },
    time::Rate,
};

use crate::data_plane;

pub const WIDTH: usize = 320;
pub const HEIGHT: usize = 240;
const BYTES_PER_PIXEL: usize = 2;
const SCANLINE_BYTES: usize = WIDTH * BYTES_PER_PIXEL;
const FRAME_BYTES: usize = WIDTH * HEIGHT * BYTES_PER_PIXEL;
const PSRAM_ALIGNMENT: usize = 32;
// Six complete RGB565 scanlines per descriptor. 320 * 2 * 6 = 3840 bytes,
// so a QVGA frame is exactly 40 descriptors with no partial tail descriptor.
const DMA_CHUNK_BYTES: usize = SCANLINE_BYTES * 6;
const SHORT_FRAME_LOG_INTERVAL: u32 = 32;

type InFlight = CameraTransfer<'static, DmaRxBuf>;

const _: () = assert!(FRAME_BYTES % DMA_CHUNK_BYTES == 0);
const _: () = assert!(DMA_CHUNK_BYTES % PSRAM_ALIGNMENT == 0);

#[repr(C, align(32))]
#[derive(Clone, Copy)]
struct AlignedBlock([u8; PSRAM_ALIGNMENT]);

pub struct Resources {
    pub lcd_cam: LCD_CAM<'static>,
    pub dma: DMA_CH2<'static>,
    pub pclk: GPIO45<'static>,
    pub vsync: GPIO46<'static>,
    pub href: GPIO38<'static>,
    pub d0: GPIO39<'static>,
    pub d1: GPIO40<'static>,
    pub d2: GPIO41<'static>,
    pub d3: GPIO42<'static>,
    pub d4: GPIO15<'static>,
    pub d5: GPIO16<'static>,
    pub d6: GPIO48<'static>,
    pub d7: GPIO47<'static>,
}

pub struct Camera {
    // `driver` + `spare_buffer` are populated only while no transfer is armed.
    // During normal streaming the driver and one DMA buffer live in `in_flight`,
    // while `display_buffer` is exclusively owned by the renderer.
    driver: Option<CameraDriver<'static>>,
    display_buffer: Option<DmaRxBuf>,
    spare_buffer: Option<DmaRxBuf>,
    in_flight: Option<InFlight>,
    short_frames: u32,
}

pub struct Frame<'a> {
    bytes: &'a [u8],
}

impl Frame<'_> {
    pub fn scanline(&self, y: usize) -> &[u8] {
        debug_assert!(y < HEIGHT);
        let start = y * SCANLINE_BYTES;
        &self.bytes[start..start + SCANLINE_BYTES]
    }
}

/// Program the GC0308 over a startup-only hardware SCCB/I2C owner.
///
/// GPIO12/GPIO11 are shared with the board's normal system I2C bus. Bootstrap
/// deliberately drops its temporary board-I2C driver, creates a fresh 100 kHz
/// hardware owner for this call, then drops it again before constructing the
/// persistent 400 kHz runtime bus. This mirrors M5Stack's CoreS3 camera bring-up.
pub fn init_sensor<I2C>(i2c: &mut I2C, delay: &mut Delay) -> Result<u8, I2C::Error>
where
    I2C: embedded_hal::i2c::I2c,
{
    gc0308::init(i2c, delay)
}

pub const EXPECTED_SENSOR_PID: u8 = gc0308::EXPECTED_PID;

pub fn init(resources: Resources) -> Camera {
    let Resources {
        lcd_cam,
        dma,
        pclk,
        vsync,
        href,
        d0,
        d1,
        d2,
        d3,
        d4,
        d5,
        d6,
        d7,
    } = resources;

    let lcd_cam = LcdCam::new(lcd_cam);
    let config = CameraConfig::default().with_frequency(Rate::from_mhz(20));
    let driver = CameraDriver::new(lcd_cam.cam, dma, config)
        .expect("Failed to configure LCD_CAM camera input")
        // CoreS3 Lite has no MCU-driven camera XCLK pin, so this deliberately
        // stays in LCD_CAM slave mode. The board provides its own 20 MHz clock.
        .with_pixel_clock(pclk)
        .with_vsync(vsync)
        .with_h_enable(href)
        .with_data0(d0)
        .with_data1(d1)
        .with_data2(d2)
        .with_data3(d3)
        .with_data4(d4)
        .with_data5(d5)
        .with_data6(d6)
        .with_data7(d7);

    let display_buffer = {
        let blocks = data_plane::leaked_filled_slice(
            FRAME_BYTES / PSRAM_ALIGNMENT,
            AlignedBlock([0; PSRAM_ALIGNMENT]),
        );
        let frame_bytes = unsafe {
            core::slice::from_raw_parts_mut(blocks.as_mut_ptr().cast::<u8>(), FRAME_BYTES)
        };
        let (rx_descriptors, _tx_descriptors) =
            esp_hal::dma_descriptors_chunk_size!(FRAME_BYTES, DMA_CHUNK_BYTES);
        DmaRxBuf::new_with_config(
            rx_descriptors,
            frame_bytes,
            ExternalBurstConfig::Size32,
        )
        .expect("Failed to construct camera display DMA buffer")
    };

    let spare_buffer = {
        let blocks = data_plane::leaked_filled_slice(
            FRAME_BYTES / PSRAM_ALIGNMENT,
            AlignedBlock([0; PSRAM_ALIGNMENT]),
        );
        let frame_bytes = unsafe {
            core::slice::from_raw_parts_mut(blocks.as_mut_ptr().cast::<u8>(), FRAME_BYTES)
        };
        let (rx_descriptors, _tx_descriptors) =
            esp_hal::dma_descriptors_chunk_size!(FRAME_BYTES, DMA_CHUNK_BYTES);
        DmaRxBuf::new_with_config(
            rx_descriptors,
            frame_bytes,
            ExternalBurstConfig::Size32,
        )
        .expect("Failed to construct camera capture DMA buffer")
    };

    Camera {
        driver: Some(driver),
        display_buffer: Some(display_buffer),
        spare_buffer: Some(spare_buffer),
        in_flight: None,
        short_frames: 0,
    }
}

impl Camera {
    fn receive(
        &mut self,
        driver: CameraDriver<'static>,
        buffer: DmaRxBuf,
    ) -> Result<InFlight, (CameraDriver<'static>, DmaRxBuf)> {
        match driver.receive(buffer) {
            Ok(transfer) => Ok(transfer),
            Err((error, driver, buffer)) => {
                log::warn!("Camera DMA start failed: {:?}", error);
                Err((driver, buffer))
            }
        }
    }

    fn wait_transfer(
        &mut self,
        transfer: InFlight,
    ) -> (Result<(), DmaError>, CameraDriver<'static>, DmaRxBuf) {
        transfer.wait()
    }

    /// Throw away the remainder of whatever frame is currently on the DVP bus,
    /// then capture one frame starting immediately after that VSYNC boundary.
    fn resynchronize(
        &mut self,
        driver: CameraDriver<'static>,
        buffer: DmaRxBuf,
    ) -> Option<(Result<(), DmaError>, CameraDriver<'static>, DmaRxBuf)> {
        let discard = match self.receive(driver, buffer) {
            Ok(transfer) => transfer,
            Err((driver, buffer)) => {
                self.driver = Some(driver);
                self.spare_buffer = Some(buffer);
                return None;
            }
        };
        let (_discard_result, driver, buffer) = self.wait_transfer(discard);

        // `wait()` returned because LCD_CAM observed VSYNC. Re-arm immediately,
        // during vertical blanking, so this transfer begins at the next frame's
        // first active pixels rather than part-way through a frame.
        let aligned = match self.receive(driver, buffer) {
            Ok(transfer) => transfer,
            Err((driver, buffer)) => {
                self.driver = Some(driver);
                self.spare_buffer = Some(buffer);
                return None;
            }
        };
        Some(self.wait_transfer(aligned))
    }

    fn report_frame_result(&mut self, result: Result<(), DmaError>, received: usize) -> bool {
        if let Err(error) = result {
            log::warn!("Camera frame DMA failed: {:?}", error);
            return false;
        }
        if received >= FRAME_BYTES {
            return true;
        }

        self.short_frames = self.short_frames.saturating_add(1);
        if self.short_frames == 1 || self.short_frames % SHORT_FRAME_LOG_INTERVAL == 0 {
            log::warn!(
                "Camera dropped short frame: {} / {} bytes (total short frames={})",
                received,
                FRAME_BYTES,
                self.short_frames
            );
        }
        false
    }

    fn arm_next(
        &mut self,
        driver: CameraDriver<'static>,
        buffer: DmaRxBuf,
    ) -> bool {
        match self.receive(driver, buffer) {
            Ok(transfer) => {
                self.in_flight = Some(transfer);
                true
            }
            Err((driver, buffer)) => {
                self.driver = Some(driver);
                self.spare_buffer = Some(buffer);
                false
            }
        }
    }

    /// Return the newest complete QVGA frame while immediately arming the other
    /// PSRAM buffer for the following frame.
    ///
    /// ESP-HAL's default LCD_CAM configuration uses VSYNC as DMA EOF. Starting a
    /// transfer at an arbitrary point therefore captures only the remainder of
    /// that sensor frame. The first capture (and any capture resumed after the
    /// in-flight transfer has already completed) is explicitly re-synchronized
    /// by discarding to VSYNC before acquiring a displayable frame.
    pub fn capture(&mut self) -> Option<Frame<'_>> {
        let (result, driver, completed_buffer) = if let Some(transfer) = self.in_flight.take() {
            // If this transfer had already finished before we got here, its VSYNC
            // boundary is stale: rendering or another view kept CPU0 away long
            // enough that re-arming now would begin in the middle of a frame.
            let boundary_is_stale = transfer.is_done();
            let (result, driver, buffer) = self.wait_transfer(transfer);
            if boundary_is_stale {
                self.resynchronize(driver, buffer)?
            } else {
                (result, driver, buffer)
            }
        } else {
            let driver = self.driver.take().expect("Camera driver missing");
            let buffer = self.spare_buffer.take().expect("Camera spare DMA buffer missing");
            self.resynchronize(driver, buffer)?
        };

        let received = completed_buffer.number_of_received_bytes();
        let complete = self.report_frame_result(result, received);

        // Start the following capture before exposing this completed buffer to
        // the renderer. The previous display buffer is now exclusively available
        // for DMA, so acquisition overlaps the 40 MHz LCD scanline transfer.
        let next_buffer = self
            .display_buffer
            .take()
            .expect("Camera display DMA buffer missing");
        let next_armed = self.arm_next(driver, next_buffer);
        self.display_buffer = Some(completed_buffer);

        if !complete || !next_armed {
            return None;
        }

        let bytes = &self.display_buffer.as_ref()?.as_slice()[..FRAME_BYTES];
        Some(Frame { bytes })
    }
}
