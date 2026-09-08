//! CPU0-owned CoreS3 Lite camera capture.
//!
//! The onboard GC0308 emits native QVGA RGB565 over its 8-bit DVP bus. LCD_CAM
//! receives frames into two permanently allocated, cache-line aligned PSRAM
//! buffers. VSYNC is used only to align the start of a capture; DMA completion
//! is driven by buffer exhaustion rather than by the next VSYNC pulse.

mod gc0308;

use esp_hal::{
    delay::Delay,
    dma::{DmaError, DmaRxBuf, ExternalBurstConfig},
    gpio::{Flex, InputConfig},
    lcd_cam::{
        LcdCam,
        cam::{
            Camera as CameraDriver, CameraTransfer, Config as CameraConfig, EofMode,
        },
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
// descriptors at its maximum compatible size: 4096 - 32 = 4064 bytes.
const DMA_CHUNK_BYTES: usize = 4096 - PSRAM_ALIGNMENT;
// Espressif's ESP32-S3 camera driver does not use VSYNC as GDMA SUC_EOF. Use
// byte-count EOF instead; the value is programmed as count - 1. The DMA keeps
// consuming descriptors after these intermediate EOF markers and finally stops
// when the frame-sized descriptor chain is exhausted.
const DMA_EOF_BYTE_LEN: u16 = u16::MAX;
const SHORT_FRAME_LOG_INTERVAL: u32 = 32;

type InFlight = CameraTransfer<'static, DmaRxBuf>;

const _: () = assert!(DMA_CHUNK_BYTES <= 4095);
const _: () = assert!(DMA_CHUNK_BYTES % PSRAM_ALIGNMENT == 0);
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
    // `driver` + `spare_buffer` are populated only while no transfer is armed.
    // During normal streaming the driver and one DMA buffer live in `in_flight`,
    // while `display_buffer` is exclusively owned by the renderer.
    driver: Option<CameraDriver<'static>>,
    display_buffer: Option<DmaRxBuf>,
    spare_buffer: Option<DmaRxBuf>,
    in_flight: Option<InFlight>,
    // Flex::peripheral_input() leaves GPIO46 readable while a frozen input
    // signal is also routed to LCD_CAM, so software can align a fresh transfer
    // to the real sensor VSYNC without unsafe pin aliasing.
    vsync: Flex<'static>,
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

    let mut vsync = Flex::new(vsync);
    vsync.apply_input_config(&InputConfig::default());
    vsync.set_input_enable(true);
    let vsync_signal = vsync.peripheral_input();

    let config = CameraConfig::default()
        .with_frequency(Rate::from_mhz(20))
        .with_eof_mode(EofMode::ByteLen(DMA_EOF_BYTE_LEN));
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
        vsync,
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

    /// Wait for the beginning of a fresh VSYNC pulse.
    ///
    /// If we arrive while VSYNC is already active, first wait for it to become
    /// inactive so we never mistake an old pulse for a new frame boundary. DMA
    /// is then armed during the new vertical blanking interval, before active
    /// HREF pixels begin.
    fn wait_for_fresh_vsync(&self) {
        while self.vsync.is_high() {
            core::hint::spin_loop();
        }
        while self.vsync.is_low() {
            core::hint::spin_loop();
        }
    }

    fn start_aligned(
        &mut self,
        driver: CameraDriver<'static>,
        buffer: DmaRxBuf,
    ) -> Result<InFlight, (CameraDriver<'static>, DmaRxBuf)> {
        self.wait_for_fresh_vsync();
        self.receive(driver, buffer)
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
        align_to_vsync: bool,
    ) -> bool {
        let result = if align_to_vsync {
            self.start_aligned(driver, buffer)
        } else {
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

    /// Return the newest complete QVGA frame while immediately arming the other
    /// PSRAM buffer for the following frame.
    ///
    /// LCD_CAM uses byte-count EOF so physical VSYNC cannot terminate a DMA
    /// chain early. A fresh stream (or one resumed after its previous transfer
    /// completed while CPU0 was away) is aligned by polling the real GPIO46
    /// VSYNC edge before arming DMA. During steady-state streaming, the next
    /// transfer is armed immediately after the previous frame-sized buffer fills,
    /// before rendering starts, so capture overlaps the 40 MHz LCD transfer.
    pub fn capture(&mut self) -> Option<Frame<'_>> {
        let (result, driver, completed_buffer, next_needs_sync) =
            if let Some(transfer) = self.in_flight.take() {
                // A transfer that finished before we returned here still contains
                // a complete aligned frame. Its completion boundary is simply too
                // old to use as the launch point for the *next* buffer.
                let next_needs_sync = transfer.is_done();
                let (result, driver, buffer) = self.wait_transfer(transfer);
                (result, driver, buffer, next_needs_sync)
            } else {
                let driver = self.driver.take().expect("Camera driver missing");
                let buffer = self.spare_buffer.take().expect("Camera spare DMA buffer missing");
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

        // Start the following capture before exposing this completed buffer to
        // the renderer. If the previous transfer had already been sitting idle,
        // or if it was incomplete, first wait for a fresh physical VSYNC edge.
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
