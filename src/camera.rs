//! CPU0-owned CoreS3 Lite camera capture.
//!
//! The onboard GC0308 emits native QVGA RGB565 over its 8-bit DVP bus. LCD_CAM
//! is a free-running source, so capture uses ESP-HAL's streaming RX buffer rather
//! than a one-shot DMA buffer. The stream is synchronized by discarding through
//! one hardware VSYNC EOF, then exactly one VSYNC-bounded frame is copied into a
//! permanently allocated PSRAM framebuffer for rendering.

mod gc0308;

use esp_hal::{
    delay::Delay,
    dma::DmaRxStreamBuf,
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

// Streaming DMA is intentionally backed by internal RAM. Eight chunks of five
// RGB565 scanlines provide enough elasticity for CPU0 to copy completed chunks
// into PSRAM while LCD_CAM continues receiving the following pixels.
const STREAM_CHUNK_BYTES: usize = SCANLINE_BYTES * 5;
const STREAM_BUFFER_BYTES: usize = STREAM_CHUNK_BYTES * 8;
const BAD_FRAME_LOG_INTERVAL: u32 = 32;

type InFlight = CameraTransfer<'static, DmaRxStreamBuf>;

const _: () = assert!(STREAM_CHUNK_BYTES <= 4095);
const _: () = assert!(STREAM_BUFFER_BYTES % STREAM_CHUNK_BYTES == 0);
const _: () = assert!(FRAME_BYTES % PSRAM_ALIGNMENT == 0);

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
    driver: Option<CameraDriver<'static>>,
    stream_buffer: Option<DmaRxStreamBuf>,
    frame_buffer: &'static mut [u8],
    bad_frames: u32,
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
    // Keep ESP-HAL's default VSYNC EOF mode. DmaRxStreamBuf is specifically
    // designed to preserve and consume multiple RX EOF boundaries while DMA
    // continues, unlike the one-shot DmaRxBuf used by the previous attempts.
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

    let frame_buffer = {
        let blocks = data_plane::leaked_filled_slice(
            FRAME_BYTES / PSRAM_ALIGNMENT,
            AlignedBlock([0; PSRAM_ALIGNMENT]),
        );
        unsafe { core::slice::from_raw_parts_mut(blocks.as_mut_ptr().cast::<u8>(), FRAME_BYTES) }
    };

    // The stream buffer and its descriptors are statically allocated in
    // DMA-capable internal memory by the macro. It is intentionally much
    // smaller than one frame because consumed descriptors are continuously
    // recycled back to GDMA.
    let stream_buffer =
        esp_hal::dma_rx_stream_buffer!(STREAM_BUFFER_BYTES, STREAM_CHUNK_BYTES);

    Camera {
        driver: Some(driver),
        stream_buffer: Some(stream_buffer),
        frame_buffer,
        bad_frames: 0,
    }
}

impl Camera {
    fn receive(
        &mut self,
        driver: CameraDriver<'static>,
        buffer: DmaRxStreamBuf,
    ) -> Result<InFlight, (CameraDriver<'static>, DmaRxStreamBuf)> {
        match driver.receive(buffer) {
            Ok(transfer) => Ok(transfer),
            Err((error, driver, buffer)) => {
                log::warn!("Camera DMA start failed: {:?}", error);
                Err((driver, buffer))
            }
        }
    }

    /// Discard the partial frame that was already in progress when DMA started.
    ///
    /// `peek_until_eof()` exposes the VSYNC-generated EOF carried by the RX
    /// descriptor. Once that EOF is consumed, the next byte belongs to a fresh
    /// camera frame. This avoids racing software against the physical VSYNC pin.
    fn discard_until_vsync(transfer: &mut InFlight) -> bool {
        loop {
            let (available, eof) = {
                let (chunk, eof) = transfer.peek_until_eof();
                (chunk.len(), eof)
            };

            if available != 0 {
                transfer.consume(available);
            }

            if eof {
                return true;
            }

            if available == 0 {
                if transfer.is_done() {
                    return false;
                }
                core::hint::spin_loop();
            }
        }
    }

    /// Copy exactly one hardware-VSYNC-bounded frame from the internal DMA
    /// stream into the persistent PSRAM framebuffer.
    ///
    /// Returns the actual byte count observed at the next VSYNC. A count other
    /// than QVGA RGB565's 153600 bytes is rejected rather than exposing a torn
    /// frame to the renderer.
    fn copy_one_frame(&mut self, transfer: &mut InFlight) -> Option<usize> {
        let mut frame_bytes = 0usize;

        loop {
            let (available, eof) = {
                let (chunk, eof) = transfer.peek_until_eof();
                let available = chunk.len();
                let copy_start = frame_bytes.min(FRAME_BYTES);
                let copy_len = available.min(FRAME_BYTES.saturating_sub(copy_start));

                if copy_len != 0 {
                    self.frame_buffer[copy_start..copy_start + copy_len]
                        .copy_from_slice(&chunk[..copy_len]);
                }

                (available, eof)
            };

            if available != 0 {
                frame_bytes = frame_bytes.saturating_add(available);
                transfer.consume(available);
            }

            if eof {
                return Some(frame_bytes);
            }

            if available == 0 {
                if transfer.is_done() {
                    return None;
                }
                core::hint::spin_loop();
            }
        }
    }

    fn report_bad_frame(&mut self, received: Option<usize>) {
        self.bad_frames = self.bad_frames.saturating_add(1);
        if self.bad_frames != 1 && self.bad_frames % BAD_FRAME_LOG_INTERVAL != 0 {
            return;
        }

        match received {
            Some(received) => log::warn!(
                "Camera VSYNC frame size mismatch: {} / {} bytes (total bad frames={})",
                received,
                FRAME_BYTES,
                self.bad_frames
            ),
            None => log::warn!(
                "Camera stream DMA stopped before the next VSYNC (total bad frames={})",
                self.bad_frames
            ),
        }
    }

    /// Capture one complete QVGA frame.
    ///
    /// The transfer starts at an arbitrary point in the free-running DVP stream.
    /// We drain through the first VSYNC EOF to establish a hardware frame
    /// boundary, then copy bytes until the following VSYNC. DMA is stopped before
    /// returning the frame, so the renderer has exclusive access to PSRAM while
    /// it pushes the image to the LCD.
    pub fn capture(&mut self) -> Option<Frame<'_>> {
        let driver = self.driver.take().expect("Camera driver missing");
        let stream_buffer = self
            .stream_buffer
            .take()
            .expect("Camera stream DMA buffer missing");

        let mut transfer = match self.receive(driver, stream_buffer) {
            Ok(transfer) => transfer,
            Err((driver, stream_buffer)) => {
                self.driver = Some(driver);
                self.stream_buffer = Some(stream_buffer);
                return None;
            }
        };

        let synchronized = Self::discard_until_vsync(&mut transfer);
        let received = if synchronized {
            self.copy_one_frame(&mut transfer)
        } else {
            None
        };

        let (driver, stream_buffer) = transfer.stop();
        self.driver = Some(driver);
        self.stream_buffer = Some(stream_buffer);

        if received != Some(FRAME_BYTES) {
            self.report_bad_frame(received);
            return None;
        }

        Some(Frame {
            bytes: &self.frame_buffer[..FRAME_BYTES],
        })
    }
}
