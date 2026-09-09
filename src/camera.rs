//! CPU0-owned CoreS3 Lite camera capture.
//!
//! The onboard GC0308 emits native QVGA RGB565 over its 8-bit DVP bus. LCD_CAM
//! is a free-running source, so capture uses ESP-HAL's streaming RX buffer. Once
//! synchronized to a hardware VSYNC boundary, the DMA stream stays active across
//! displayed frames and the UI consumes one camera scanline at a time. This lets
//! camera DMA overlap LCD SPI-DMA instead of staging every frame through PSRAM.

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

pub const WIDTH: usize = 320;
pub const HEIGHT: usize = 240;
const BYTES_PER_PIXEL: usize = 2;
const SCANLINE_BYTES: usize = WIDTH * BYTES_PER_PIXEL;

// Streaming DMA is intentionally backed by internal RAM. Eight chunks of five
// RGB565 scanlines are enough to bridge the short CPU0 bookkeeping gap between
// displayed frames while descriptors are continuously recycled as scanlines are
// consumed by the renderer.
const STREAM_CHUNK_BYTES: usize = SCANLINE_BYTES * 5;
const STREAM_BUFFER_BYTES: usize = STREAM_CHUNK_BYTES * 8;
const BAD_FRAME_LOG_INTERVAL: u32 = 32;

type InFlight = CameraTransfer<'static, DmaRxStreamBuf>;

const _: () = assert!(STREAM_CHUNK_BYTES <= 4095);
const _: () = assert!(STREAM_BUFFER_BYTES % STREAM_CHUNK_BYTES == 0);

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
    // When the Camera view is active, `in_flight` owns the LCD_CAM driver and
    // stream buffer continuously across frames. If the view is left long enough
    // for the small stream ring to fill, the stopped transfer is recovered and
    // re-synchronized the next time `begin_frame` is called.
    driver: Option<CameraDriver<'static>>,
    stream_buffer: Option<DmaRxStreamBuf>,
    in_flight: Option<InFlight>,
    line_buffer: [u8; SCANLINE_BYTES],
    bad_frames: u32,
}

/// One VSYNC-bounded streaming frame.
///
/// Scanlines are consumed in sensor order. The GC0308 is configured for the
/// board's 180-degree mounting in hardware, so no framebuffer reversal is needed
/// in the UI hot path.
pub struct Frame<'a> {
    camera: &'a mut Camera,
    transfer: Option<InFlight>,
    line_index: usize,
    frame_bytes: usize,
    eof_seen: bool,
    valid: bool,
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
    // Keep ESP-HAL's VSYNC EOF mode: DmaRxStreamBuf preserves EOF boundaries
    // while DMA continues, so each sensor VSYNC cleanly separates frames.
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

    // The stream buffer and descriptors are statically allocated in DMA-capable
    // internal memory. Unlike the previous implementation, no full QVGA PSRAM
    // framebuffer is needed: pixels move camera DMA -> one scanline -> LCD DMA.
    let stream_buffer =
        esp_hal::dma_rx_stream_buffer!(STREAM_BUFFER_BYTES, STREAM_CHUNK_BYTES);

    Camera {
        driver: Some(driver),
        stream_buffer: Some(stream_buffer),
        in_flight: None,
        line_buffer: [0; SCANLINE_BYTES],
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

    fn start_stream(&mut self) -> Option<InFlight> {
        let driver = self.driver.take().expect("Camera driver missing");
        let stream_buffer = self
            .stream_buffer
            .take()
            .expect("Camera stream DMA buffer missing");

        match self.receive(driver, stream_buffer) {
            Ok(transfer) => Some(transfer),
            Err((driver, stream_buffer)) => {
                self.driver = Some(driver);
                self.stream_buffer = Some(stream_buffer);
                None
            }
        }
    }

    fn recover_stopped(&mut self, transfer: InFlight) {
        let (driver, stream_buffer) = transfer.stop();
        self.driver = Some(driver);
        self.stream_buffer = Some(stream_buffer);
        self.in_flight = None;
    }

    /// Discard the partial frame that was already in progress when DMA starts.
    /// Once the first VSYNC EOF is consumed, the next byte belongs to a complete
    /// fresh camera frame.
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

    /// Fill one native 320-pixel RGB565 scanline from the active stream.
    /// Returns `(bytes_read, eof_consumed)`.
    fn read_scanline_bytes(
        transfer: &mut InFlight,
        out: &mut [u8; SCANLINE_BYTES],
    ) -> (usize, bool) {
        let mut filled = 0usize;

        while filled < SCANLINE_BYTES {
            let (available, take, eof) = {
                let (chunk, eof) = transfer.peek_until_eof();
                let available = chunk.len();
                let take = available.min(SCANLINE_BYTES - filled);
                if take != 0 {
                    out[filled..filled + take].copy_from_slice(&chunk[..take]);
                }
                (available, take, eof)
            };

            if take != 0 {
                transfer.consume(take);
                filled += take;
            }

            // `peek_until_eof` ends its returned slice at the EOF descriptor. We
            // have consumed that boundary only when all bytes from the slice were
            // consumed in this iteration.
            if eof && take == available {
                return (filled, true);
            }

            if available == 0 {
                if transfer.is_done() {
                    return (filled, false);
                }
                core::hint::spin_loop();
            }
        }

        (filled, false)
    }

    fn report_bad_frame(&mut self, received: usize) {
        self.bad_frames = self.bad_frames.saturating_add(1);
        if self.bad_frames == 1 || self.bad_frames % BAD_FRAME_LOG_INTERVAL == 0 {
            log::warn!(
                "Camera VSYNC frame size mismatch: {} / {} bytes (total bad frames={})",
                received,
                WIDTH * HEIGHT * BYTES_PER_PIXEL,
                self.bad_frames
            );
        }
    }

    /// Start consuming the next complete camera frame without stopping the
    /// free-running DMA stream between successful frames.
    pub fn begin_frame(&mut self) -> Option<Frame<'_>> {
        let mut needs_sync = false;

        let mut transfer = if let Some(transfer) = self.in_flight.take() {
            if transfer.is_done() {
                self.recover_stopped(transfer);
                needs_sync = true;
                self.start_stream()?
            } else {
                transfer
            }
        } else {
            needs_sync = true;
            self.start_stream()?
        };

        if needs_sync && !Self::discard_until_vsync(&mut transfer) {
            self.recover_stopped(transfer);
            self.report_bad_frame(0);
            return None;
        }

        Some(Frame {
            camera: self,
            transfer: Some(transfer),
            line_index: 0,
            frame_bytes: 0,
            eof_seen: false,
            valid: true,
        })
    }
}

impl Frame<'_> {
    /// Return the next native camera scanline.
    ///
    /// The slice is valid until the next call. A valid QVGA frame must deliver
    /// exactly 240 complete 640-byte scanlines and expose VSYNC EOF on the final
    /// one. Any other boundary is rejected and forces re-synchronization.
    pub fn next_scanline(&mut self) -> Option<&[u8]> {
        if !self.valid || self.line_index >= HEIGHT {
            return None;
        }

        let transfer = self.transfer.as_mut()?;
        let (read, eof) = Camera::read_scanline_bytes(transfer, &mut self.camera.line_buffer);
        self.frame_bytes = self.frame_bytes.saturating_add(read);

        if read != SCANLINE_BYTES {
            self.valid = false;
            return None;
        }

        self.line_index += 1;
        if eof {
            self.eof_seen = true;
            if self.line_index != HEIGHT {
                self.valid = false;
                return None;
            }
        } else if self.line_index == HEIGHT {
            self.valid = false;
            return None;
        }

        Some(&self.camera.line_buffer)
    }

    /// Complete the frame and hand the still-running stream back to `Camera`.
    /// Successful frames preserve DMA continuity; invalid frames stop the stream
    /// so the next call can synchronize from a known VSYNC boundary.
    pub fn finish(mut self) -> bool {
        let expected = WIDTH * HEIGHT * BYTES_PER_PIXEL;
        let valid = self.valid
            && self.line_index == HEIGHT
            && self.eof_seen
            && self.frame_bytes == expected;

        let transfer = self.transfer.take().expect("Camera frame transfer missing");

        if valid && !transfer.is_done() {
            self.camera.in_flight = Some(transfer);
        } else {
            self.camera.recover_stopped(transfer);
            if !valid {
                self.camera.report_bad_frame(self.frame_bytes);
            }
        }

        valid
    }
}

impl Drop for Frame<'_> {
    fn drop(&mut self) {
        // `finish` clears this option after either preserving or recovering the
        // transfer. If a caller abandons a frame early, recover ownership here so
        // the Camera service cannot lose its LCD_CAM/DMA resources.
        if let Some(transfer) = self.transfer.take() {
            self.camera.recover_stopped(transfer);
        }
    }
}
