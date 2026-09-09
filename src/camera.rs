//! CPU0-owned CoreS3 Lite camera capture.
//!
//! The onboard GC0308 emits native QVGA RGB565 over its 8-bit DVP bus. Capture
//! uses two PSRAM-backed DMA buffers: one immutable complete frame is rendered
//! while LCD_CAM fills the other. Buffers are swapped only after a VSYNC-bounded
//! transfer completes, so the LCD never reads scanlines that the camera is still
//! modifying.

mod gc0308;

use esp_hal::{
    delay::Delay,
    dma::{DmaError, DmaRxBuf, ExternalBurstConfig},
    gpio::{Flex, InputConfig},
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

// DmaRxBuf::new_with_config(..., ExternalBurstConfig::Size32) relinks PSRAM RX
// descriptors at 4096 - 32 = 4064 bytes. Allocate one extra scanline beyond the
// visible QVGA frame so the descriptor chain cannot exhaust at exactly 153600
// bytes. The aligned transfer therefore ends on the GC0308's real VSYNC EOF,
// which gives us a trustworthy frame boundary for ping-pong buffer swaps.
const DMA_CHUNK_BYTES: usize = 4096 - PSRAM_ALIGNMENT;
const DMA_GUARD_BYTES: usize = SCANLINE_BYTES;
const DMA_BUFFER_BYTES: usize = FRAME_BYTES + DMA_GUARD_BYTES;
const BAD_FRAME_LOG_INTERVAL: u32 = 32;

type InFlight = CameraTransfer<'static, DmaRxBuf>;

const _: () = assert!(DMA_CHUNK_BYTES <= 4095);
const _: () = assert!(DMA_CHUNK_BYTES % PSRAM_ALIGNMENT == 0);
const _: () = assert!(FRAME_BYTES % PSRAM_ALIGNMENT == 0);
const _: () = assert!(DMA_BUFFER_BYTES % PSRAM_ALIGNMENT == 0);

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
    // During normal Camera-view streaming, one buffer is owned by `in_flight`
    // and filled by DMA while `display_buffer` is immutable for the renderer.
    // `driver` + `spare_buffer` are populated only when no transfer is armed.
    driver: Option<CameraDriver<'static>>,
    display_buffer: Option<DmaRxBuf>,
    spare_buffer: Option<DmaRxBuf>,
    in_flight: Option<InFlight>,
    // Keep GPIO46 readable while also routing it to LCD_CAM. Physical VSYNC is
    // only needed to recover a fresh frame start after entering Camera or after
    // the renderer has fallen behind a completed transfer.
    vsync: Flex<'static>,
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

    let mut vsync = Flex::new(vsync);
    vsync.apply_input_config(&InputConfig::default());
    vsync.set_input_enable(true);
    let vsync_signal = vsync.peripheral_input();

    // ESP-HAL's default camera EOF mode is VSYNC. The extra guard scanline in
    // each DmaRxBuf ensures VSYNC, rather than descriptor exhaustion, is what
    // terminates a correctly aligned QVGA transfer.
    let config = CameraConfig::default().with_frequency(Rate::from_mhz(20));
    let driver = CameraDriver::new(lcd_cam.cam, dma, config)
        .expect("Failed to configure LCD_CAM camera input")
        // CoreS3 Lite has no MCU-driven camera XCLK pin, so this deliberately
        // stays in LCD_CAM slave mode. The board provides its own 20 MHz clock.
        .with_pixel_clock(pclk)
        .with_vsync(vsync_signal)
        .with_h_enable(href)
        .with_data0(d0)
        .with_data1(d1)
        .with_data2(d2)
        .with_data3(d3)
        .with_data4(d4)
        .with_data5(d5)
        .with_data6(d6)
        .with_data7(d7);

    // Expand the descriptor macro separately for each PSRAM buffer. Each macro
    // expansion owns distinct static descriptor storage; sharing one expansion
    // between both ping-pong buffers would make the second initialization reuse
    // the first buffer's DMA descriptors.
    let display_buffer = {
        let blocks = data_plane::leaked_filled_slice(
            DMA_BUFFER_BYTES / PSRAM_ALIGNMENT,
            AlignedBlock([0; PSRAM_ALIGNMENT]),
        );
        let bytes = unsafe {
            core::slice::from_raw_parts_mut(blocks.as_mut_ptr().cast::<u8>(), DMA_BUFFER_BYTES)
        };
        let (rx_descriptors, _tx_descriptors) =
            esp_hal::dma_descriptors_chunk_size!(DMA_BUFFER_BYTES, DMA_CHUNK_BYTES);
        DmaRxBuf::new_with_config(rx_descriptors, bytes, ExternalBurstConfig::Size32)
            .expect("Failed to construct camera display DMA buffer")
    };

    let spare_buffer = {
        let blocks = data_plane::leaked_filled_slice(
            DMA_BUFFER_BYTES / PSRAM_ALIGNMENT,
            AlignedBlock([0; PSRAM_ALIGNMENT]),
        );
        let bytes = unsafe {
            core::slice::from_raw_parts_mut(blocks.as_mut_ptr().cast::<u8>(), DMA_BUFFER_BYTES)
        };
        let (rx_descriptors, _tx_descriptors) =
            esp_hal::dma_descriptors_chunk_size!(DMA_BUFFER_BYTES, DMA_CHUNK_BYTES);
        DmaRxBuf::new_with_config(rx_descriptors, bytes, ExternalBurstConfig::Size32)
            .expect("Failed to construct camera capture DMA buffer")
    };

    Camera {
        driver: Some(driver),
        display_buffer: Some(display_buffer),
        spare_buffer: Some(spare_buffer),
        in_flight: None,
        vsync,
        bad_frames: 0,
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

    /// Wait until the end of a physical VSYNC pulse, which is the start of a
    /// fresh active frame for the GC0308 configuration used on CoreS3 Lite.
    ///
    /// If called during active video (VSYNC low), wait for the next pulse first.
    /// If called while VSYNC is already high, use that current boundary rather
    /// than unnecessarily skipping another frame.
    fn wait_for_fresh_frame_start(&self) {
        if self.vsync.is_low() {
            while self.vsync.is_low() {
                core::hint::spin_loop();
            }
        }
        while self.vsync.is_high() {
            core::hint::spin_loop();
        }
    }

    fn start_aligned(
        &mut self,
        driver: CameraDriver<'static>,
        buffer: DmaRxBuf,
    ) -> Result<InFlight, (CameraDriver<'static>, DmaRxBuf)> {
        self.wait_for_fresh_frame_start();
        self.receive(driver, buffer)
    }

    fn report_frame_result(&mut self, result: Result<(), DmaError>, received: usize) -> bool {
        if let Err(error) = result {
            log::warn!("Camera frame DMA failed: {:?}", error);
            return false;
        }
        if received == FRAME_BYTES {
            return true;
        }

        self.bad_frames = self.bad_frames.saturating_add(1);
        if self.bad_frames == 1 || self.bad_frames % BAD_FRAME_LOG_INTERVAL == 0 {
            log::warn!(
                "Camera VSYNC frame size mismatch: {} / {} bytes (total bad frames={})",
                received,
                FRAME_BYTES,
                self.bad_frames
            );
        }
        false
    }

    fn arm_next(
        &mut self,
        driver: CameraDriver<'static>,
        buffer: DmaRxBuf,
        align_to_vsync: bool,
    ) -> bool {
        let result = if align_to_vsync {
            self.start_aligned(driver, buffer)
        } else {
            // A non-stale successful transfer ended on the VSYNC EOF we just
            // observed. Re-arm immediately so capture of the next frame overlaps
            // LCD rendering of the completed buffer.
            self.receive(driver, buffer)
        };

        match result {
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

    /// Return one immutable complete QVGA frame while immediately arming DMA
    /// into the other PSRAM buffer for the following frame.
    ///
    /// The extra guard scanline makes a normal transfer end at VSYNC rather than
    /// at descriptor exhaustion. If CPU0 returns after the in-flight transfer has
    /// already been sitting complete, only the *next* capture is re-aligned to a
    /// fresh physical VSYNC; the completed frame itself is still valid.
    pub fn capture(&mut self) -> Option<Frame<'_>> {
        let (result, driver, completed_buffer, next_needs_sync) =
            if let Some(transfer) = self.in_flight.take() {
                let next_needs_sync = transfer.is_done();
                let (result, driver, buffer) = self.wait_transfer(transfer);
                (result, driver, buffer, next_needs_sync)
            } else {
                let driver = self.driver.take().expect("Camera driver missing");
                let buffer = self
                    .spare_buffer
                    .take()
                    .expect("Camera spare DMA buffer missing");
                let transfer = match self.start_aligned(driver, buffer) {
                    Ok(transfer) => transfer,
                    Err((driver, buffer)) => {
                        self.driver = Some(driver);
                        self.spare_buffer = Some(buffer);
                        return None;
                    }
                };
                let (result, driver, buffer) = self.wait_transfer(transfer);
                (result, driver, buffer, false)
            };

        let received = completed_buffer.number_of_received_bytes();
        let complete = self.report_frame_result(result, received);

        let next_buffer = self
            .display_buffer
            .take()
            .expect("Camera display DMA buffer missing");
        let next_armed = self.arm_next(driver, next_buffer, next_needs_sync || !complete);
        self.display_buffer = Some(completed_buffer);

        if !complete || !next_armed {
            return None;
        }

        let bytes = &self.display_buffer.as_ref()?.as_slice()[..FRAME_BYTES];
        Some(Frame { bytes })
    }
}
