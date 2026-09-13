//! CPU0-owned LCD_CAM/DMA camera capture pipeline.
//!
//! The onboard GC0308 emits native QVGA RGB565 over its 8-bit DVP bus. LCD_CAM
//! stays free-running through ESP-HAL's streaming RX buffer. Two PSRAM frame
//! buffers decouple sensor timing from LCD timing: one complete frame is rendered
//! while CPU0 drains the next frame from the small internal DMA ring during SPI
//! DMA wait time. This preserves hardware-VSYNC framing without pacing LCD writes
//! directly from live camera scanlines.

use esp_hal::{
    dma::DmaRxStreamBuf,
    lcd_cam::{
        LcdCam,
        cam::{Camera as CameraDriver, CameraTransfer, Config as CameraConfig},
    },
    peripherals::{
        DMA_CH2, GPIO15, GPIO16, GPIO38, GPIO39, GPIO40, GPIO41, GPIO42, GPIO45, GPIO46, GPIO47,
        GPIO48, LCD_CAM,
    },
    time::Rate,
};

use crate::support::memory::storage;

pub const WIDTH: usize = 320;
pub const HEIGHT: usize = 240;
const BYTES_PER_PIXEL: usize = 2;
const SCANLINE_BYTES: usize = WIDTH * BYTES_PER_PIXEL;
const FRAME_BYTES: usize = WIDTH * HEIGHT * BYTES_PER_PIXEL;
const PSRAM_ALIGNMENT: usize = 32;

// Keep the exact small internal stream-ring geometry that was already proven on
// hardware. The renderer pumps this ring while each LCD DMA batch is in flight,
// so it no longer needs to buffer most of a frame in scarce DRAM.
const STREAM_CHUNK_BYTES: usize = SCANLINE_BYTES * 5;
const STREAM_BUFFER_BYTES: usize = STREAM_CHUNK_BYTES * 8;
const BAD_FRAME_LOG_INTERVAL: u32 = 32;

type InFlight = CameraTransfer<'static, DmaRxStreamBuf>;

const _: () = assert!(STREAM_CHUNK_BYTES <= 4095);
const _: () = assert!(STREAM_BUFFER_BYTES.is_multiple_of(STREAM_CHUNK_BYTES));
const _: () = assert!(FRAME_BYTES.is_multiple_of(PSRAM_ALIGNMENT));

#[repr(C, align(32))]
#[derive(Clone, Copy)]
struct AlignedBlock([u8; PSRAM_ALIGNMENT]);

pub(crate) struct Resources {
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

enum CaptureState {
    Stopped {
        driver: CameraDriver<'static>,
        stream_buffer: DmaRxStreamBuf,
    },
    Streaming(InFlight),
}

pub struct Camera {
    // `None` exists only transiently while a method moves the concrete state.
    // Between method calls, this always contains exactly one legal ownership state.
    state: Option<CaptureState>,
    display_buffer: &'static mut [u8],
    capture_buffer: &'static mut [u8],
    display_ready: bool,
    capture_bytes: usize,
    capture_done: bool,
    capture_valid: bool,
    bad_frames: u32,
}

/// One frozen camera frame being presented while the following frame is captured.
pub struct Frame<'a> {
    camera: &'a mut Camera,
}

impl Frame<'_> {
    pub fn scanline(&self, y: usize) -> &[u8] {
        debug_assert!(y < HEIGHT);
        let start = y * SCANLINE_BYTES;
        &self.camera.display_buffer[start..start + SCANLINE_BYTES]
    }

    /// Drain every camera byte currently available without blocking for new data.
    /// The display transport calls this while SPI DMA is already transmitting the
    /// previous LCD batch, turning what used to be CPU idle time into next-frame
    /// capture work.
    pub fn pump(&mut self) {
        self.camera.pump_capture_available();
    }

    /// Finish receiving the following VSYNC-bounded frame and prepare the next
    /// presentation frame. Capture faults are handled internally and cause the
    /// next `begin_frame` call to re-prime from a fresh VSYNC boundary.
    pub fn finish(self) {
        let _ = self.camera.complete_capture_and_swap();
    }
}

fn alloc_frame_buffer() -> &'static mut [u8] {
    let blocks = storage::leaked_filled_slice(
        FRAME_BYTES / PSRAM_ALIGNMENT,
        AlignedBlock([0; PSRAM_ALIGNMENT]),
    );
    // SAFETY: `AlignedBlock` is `repr(C, align(32))` and contains exactly one
    // `[u8; 32]` with no padding inside the block. The compile-time divisibility
    // assertion makes the allocated block span exactly `FRAME_BYTES`; the leaked
    // slice gives this byte view the same device lifetime and unique mutability.
    unsafe { core::slice::from_raw_parts_mut(blocks.as_mut_ptr().cast::<u8>(), FRAME_BYTES) }
}

pub(crate) fn init(resources: Resources) -> Camera {
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
    // Keep ESP-HAL's VSYNC EOF mode. DmaRxStreamBuf preserves EOF boundaries
    // while descriptors are recycled, unlike the unreliable one-shot PSRAM DMA
    // framing experiments.
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

    let stream_buffer = esp_hal::dma_rx_stream_buffer!(STREAM_BUFFER_BYTES, STREAM_CHUNK_BYTES);

    Camera {
        state: Some(CaptureState::Stopped {
            driver,
            stream_buffer,
        }),
        display_buffer: alloc_frame_buffer(),
        capture_buffer: alloc_frame_buffer(),
        display_ready: false,
        capture_bytes: 0,
        capture_done: false,
        capture_valid: false,
        bad_frames: 0,
    }
}

impl Camera {
    fn take_state(&mut self) -> CaptureState {
        self.state.take().expect("Camera capture state missing")
    }

    fn receive(
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

    fn start_stream(&mut self) -> bool {
        let state = self.take_state();
        let CaptureState::Stopped {
            driver,
            stream_buffer,
        } = state
        else {
            self.state = Some(state);
            panic!("Camera stream started while capture was already running");
        };

        match Self::receive(driver, stream_buffer) {
            Ok(transfer) => {
                self.state = Some(CaptureState::Streaming(transfer));
                true
            }
            Err((driver, stream_buffer)) => {
                self.state = Some(CaptureState::Stopped {
                    driver,
                    stream_buffer,
                });
                false
            }
        }
    }

    fn stop_stream(&mut self) {
        let state = self.take_state();
        self.state = Some(match state {
            CaptureState::Stopped { .. } => state,
            CaptureState::Streaming(transfer) => {
                let (driver, stream_buffer) = transfer.stop();
                CaptureState::Stopped {
                    driver,
                    stream_buffer,
                }
            }
        });
    }

    /// Discard the partial sensor frame already in progress when DMA is started.
    /// Once this hardware VSYNC EOF is consumed, the next unread byte is the
    /// beginning of a complete fresh frame.
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

    fn reset_capture_state(&mut self) {
        self.capture_bytes = 0;
        self.capture_done = false;
        self.capture_valid = false;
    }

    fn restart_stream_aligned(&mut self) -> bool {
        self.stop_stream();
        if !self.start_stream() {
            return false;
        }

        let state = self.take_state();
        let CaptureState::Streaming(mut transfer) = state else {
            self.state = Some(state);
            panic!("Camera capture state did not enter streaming mode");
        };

        if !Self::discard_until_vsync(&mut transfer) {
            let (driver, stream_buffer) = transfer.stop();
            self.state = Some(CaptureState::Stopped {
                driver,
                stream_buffer,
            });
            return false;
        }

        self.state = Some(CaptureState::Streaming(transfer));
        true
    }

    /// Copy all bytes that DMA has already delivered into the capture PSRAM
    /// buffer, stopping at the next hardware VSYNC EOF. This method never waits
    /// for more camera data; callers may invoke it repeatedly inside LCD DMA
    /// polling loops.
    fn pump_capture_available(&mut self) {
        if self.capture_done {
            return;
        }

        let stream_stopped = matches!(self.state, Some(CaptureState::Stopped { .. }));
        if stream_stopped && (self.capture_bytes != 0 || !self.restart_stream_aligned()) {
            self.capture_done = true;
            self.capture_valid = false;
            return;
        }

        let state = self.take_state();
        let CaptureState::Streaming(mut transfer) = state else {
            self.state = Some(state);
            panic!("Camera capture pump requires a running stream");
        };

        loop {
            let (available, eof) = {
                let (chunk, eof) = transfer.peek_until_eof();
                let available = chunk.len();
                let copy_start = self.capture_bytes.min(FRAME_BYTES);
                let copy_len = available.min(FRAME_BYTES.saturating_sub(copy_start));

                if copy_len != 0 {
                    self.capture_buffer[copy_start..copy_start + copy_len]
                        .copy_from_slice(&chunk[..copy_len]);
                }

                (available, eof)
            };

            if available != 0 {
                self.capture_bytes = self.capture_bytes.saturating_add(available);
                transfer.consume(available);
            }

            if eof {
                self.capture_done = true;
                self.capture_valid = self.capture_bytes == FRAME_BYTES;
                break;
            }

            if available == 0 {
                break;
            }
        }

        if transfer.is_done() {
            let stopped_before_eof = !self.capture_done;
            let (driver, stream_buffer) = transfer.stop();
            self.state = Some(CaptureState::Stopped {
                driver,
                stream_buffer,
            });
            if stopped_before_eof {
                self.capture_done = true;
                self.capture_valid = false;
            }
        } else {
            self.state = Some(CaptureState::Streaming(transfer));
        }
    }

    fn report_bad_frame(&mut self, received: usize) {
        self.bad_frames = self.bad_frames.saturating_add(1);
        if self.bad_frames == 1 || self.bad_frames.is_multiple_of(BAD_FRAME_LOG_INTERVAL) {
            log::warn!(
                "Camera VSYNC frame size mismatch: {} / {} bytes (total bad frames={})",
                received,
                FRAME_BYTES,
                self.bad_frames
            );
        }
    }

    /// Wait only for the not-yet-arrived tail of the frame being captured during
    /// LCD rendering, validate its VSYNC byte count, and swap PSRAM roles.
    fn complete_capture_and_swap(&mut self) -> bool {
        while !self.capture_done {
            self.pump_capture_available();
            if !self.capture_done {
                if matches!(self.state, Some(CaptureState::Stopped { .. })) {
                    break;
                }
                core::hint::spin_loop();
            }
        }

        if !self.capture_done || !self.capture_valid {
            self.report_bad_frame(self.capture_bytes);
            self.display_ready = false;
            self.reset_capture_state();
            return false;
        }

        core::mem::swap(&mut self.display_buffer, &mut self.capture_buffer);
        self.display_ready = true;
        self.reset_capture_state();
        true
    }

    /// Prime the first frozen frame after boot, a camera-view re-entry, or a
    /// stream fault. Steady-state frames do not use this blocking full-frame path;
    /// they are captured concurrently while the prior frame is sent to the LCD.
    fn prime_display_frame(&mut self) -> bool {
        self.display_ready = false;
        self.reset_capture_state();

        if !self.restart_stream_aligned() {
            self.report_bad_frame(0);
            return false;
        }

        self.complete_capture_and_swap()
    }

    /// Stop any free-running camera DMA when Camera view is no longer presented.
    /// Re-entry will prime from a fresh VSYNC boundary instead of consuming a
    /// stale partial frame that accumulated while another screen was visible.
    pub fn pause(&mut self) {
        self.stop_stream();
        self.display_ready = false;
        self.reset_capture_state();
    }

    /// Begin presenting the current frozen frame. The returned object also owns
    /// the capture pump used by the LCD transport while SPI DMA is in flight.
    pub fn begin_frame(&mut self) -> Option<Frame<'_>> {
        if !self.display_ready && !self.prime_display_frame() {
            return None;
        }

        Some(Frame { camera: self })
    }
}
