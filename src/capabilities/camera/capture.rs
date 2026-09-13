//! CPU0-owned LCD_CAM/DMA camera capture pipeline.
//!
//! The GC0308 emits QVGA RGB565 over its 8-bit DVP bus. LCD_CAM streams it
//! into a small internal DMA ring. Two PSRAM frame buffers decouple sensor
//! timing from LCD timing: one complete frame is shown while CPU0 drains the
//! next frame from the ring during LCD DMA wait time. Hardware VSYNC marks
//! frame boundaries.

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
use log::warn;

use crate::{capabilities::display::ScanlineSource, support::memory::storage};

pub const WIDTH: usize = 320;
pub const HEIGHT: usize = 240;
const BYTES_PER_PIXEL: usize = 2;
const SCANLINE_BYTES: usize = WIDTH * BYTES_PER_PIXEL;
const FRAME_BYTES: usize = WIDTH * HEIGHT * BYTES_PER_PIXEL;

/// The internal DMA ring is small; the renderer drains it while each LCD DMA
/// batch is in flight, so most of the frame never has to sit in scarce DRAM.
const STREAM_CHUNK_BYTES: usize = SCANLINE_BYTES * 5;
const STREAM_BUFFER_BYTES: usize = STREAM_CHUNK_BYTES * 8;
const BAD_FRAME_LOG_INTERVAL: u32 = 32;

const _: () = assert!(STREAM_CHUNK_BYTES <= esp_hal::dma::CHUNK_SIZE);
const _: () = assert!(STREAM_BUFFER_BYTES.is_multiple_of(STREAM_CHUNK_BYTES));

type InFlight = CameraTransfer<'static, DmaRxStreamBuf>;

pub(crate) struct Resources {
    pub(crate) lcd_cam: LCD_CAM<'static>,
    pub(crate) dma: DMA_CH2<'static>,
    pub(crate) pclk: GPIO45<'static>,
    pub(crate) vsync: GPIO46<'static>,
    pub(crate) href: GPIO38<'static>,
    pub(crate) d0: GPIO39<'static>,
    pub(crate) d1: GPIO40<'static>,
    pub(crate) d2: GPIO41<'static>,
    pub(crate) d3: GPIO42<'static>,
    pub(crate) d4: GPIO15<'static>,
    pub(crate) d5: GPIO16<'static>,
    pub(crate) d6: GPIO48<'static>,
    pub(crate) d7: GPIO47<'static>,
}

enum Stream {
    Stopped {
        driver: CameraDriver<'static>,
        buffer: DmaRxStreamBuf,
    },
    Running(InFlight),
}

impl Stream {
    fn stopped(self) -> Self {
        match self {
            Self::Stopped { .. } => self,
            Self::Running(transfer) => {
                let (driver, buffer) = transfer.stop();
                Self::Stopped { driver, buffer }
            }
        }
    }
}

/// How far the frame going into `capture_buffer` has come.
#[derive(Clone, Copy)]
enum Progress {
    /// Bytes copied so far; VSYNC not seen yet.
    Filling(usize),
    /// VSYNC seen after this many bytes. Valid only if it is a whole frame.
    Complete(usize),
}

enum CaptureError {
    DmaStart,
    StreamEnded,
    BadLength(usize),
}

impl core::fmt::Display for CaptureError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::DmaStart => write!(f, "DMA start failed"),
            Self::StreamEnded => write!(f, "DMA stopped before VSYNC"),
            Self::BadLength(bytes) => write!(f, "{bytes} of {FRAME_BYTES} bytes before VSYNC"),
        }
    }
}

pub struct Camera {
    // `None` only while a method moves the stream between states.
    stream: Option<Stream>,
    display_buffer: &'static mut [u8],
    capture_buffer: &'static mut [u8],
    display_ready: bool,
    progress: Progress,
    bad_frames: u32,
}

/// One frozen camera frame being shown while the following frame is captured.
pub struct Frame<'a> {
    camera: &'a mut Camera,
}

impl Frame<'_> {
    /// Big-endian RGB565 bytes of row `y`.
    pub fn scanline(&self, y: usize) -> &[u8] {
        debug_assert!(y < HEIGHT);
        let start = y * SCANLINE_BYTES;
        &self.camera.display_buffer[start..start + SCANLINE_BYTES]
    }

    /// Copy whatever the sensor has delivered of the next frame, without
    /// waiting. Call this whenever the CPU would otherwise idle, for example
    /// while the display DMA is busy.
    pub fn pump(&mut self) {
        self.camera.pump();
    }

    /// Wait for the rest of the next frame and make it the frame to show.
    ///
    /// Blocks until the sensor's next VSYNC, at most one frame period. A
    /// capture fault is logged and the following `begin_frame` re-syncs.
    pub fn finish(self) {
        if let Err(error) = self.camera.finish_capture() {
            self.camera.report_bad_frame(error);
        }
    }
}

/// Rendering a `Frame` straight onto a surface shows the leftmost pixels of
/// each row when the surface is narrower than the sensor image.
impl ScanlineSource for Frame<'_> {
    fn fill_row(&mut self, y: usize, row: &mut [u8]) {
        row.copy_from_slice(&self.scanline(y)[..row.len()]);
    }

    fn while_transferring(&mut self) {
        self.pump();
    }
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
    // The board supplies the sensor's 20 MHz clock itself, so LCD_CAM runs as
    // a slave without an XCLK pin. VSYNC EOF mode keeps frame boundaries
    // visible in the stream buffer.
    let config = CameraConfig::default().with_frequency(Rate::from_mhz(20));
    let driver = CameraDriver::new(lcd_cam.cam, dma, config)
        .expect("LCD_CAM camera configuration is valid")
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

    let buffer = esp_hal::dma_rx_stream_buffer!(STREAM_BUFFER_BYTES, STREAM_CHUNK_BYTES);

    Camera {
        stream: Some(Stream::Stopped { driver, buffer }),
        display_buffer: storage::leaked_filled_slice(FRAME_BYTES, 0),
        capture_buffer: storage::leaked_filled_slice(FRAME_BYTES, 0),
        display_ready: false,
        progress: Progress::Filling(0),
        bad_frames: 0,
    }
}

impl Camera {
    /// Stop capturing. The next `begin_frame` re-syncs to a fresh VSYNC
    /// instead of showing a stale partial frame.
    pub fn pause(&mut self) {
        self.stop_stream();
        self.display_ready = false;
        self.progress = Progress::Filling(0);
    }

    /// Start showing the current frame.
    ///
    /// The first call after boot or `pause` blocks for up to two frame periods
    /// to capture a complete frame. `None` when the sensor delivers nothing.
    pub fn begin_frame(&mut self) -> Option<Frame<'_>> {
        if !self.display_ready
            && let Err(error) = self.prime()
        {
            self.report_bad_frame(error);
            return None;
        }
        Some(Frame { camera: self })
    }

    fn stop_stream(&mut self) {
        self.stream = self.stream.take().map(Stream::stopped);
    }

    fn is_stopped(&self) -> bool {
        matches!(self.stream, Some(Stream::Stopped { .. }))
    }

    /// Restart DMA and discard the partial frame in progress, so the next
    /// unread byte begins a fresh frame.
    fn start_stream_aligned(&mut self) -> Result<(), CaptureError> {
        self.stop_stream();
        let Some(Stream::Stopped { driver, buffer }) = self.stream.take() else {
            unreachable!("stream was stopped above");
        };

        let mut transfer = match driver.receive(buffer) {
            Ok(transfer) => transfer,
            Err((error, driver, buffer)) => {
                warn!("Camera DMA start failed: {:?}", error);
                self.stream = Some(Stream::Stopped { driver, buffer });
                return Err(CaptureError::DmaStart);
            }
        };

        let synced = loop {
            let (chunk, eof) = transfer.peek_until_eof();
            let available = chunk.len();
            if available != 0 {
                transfer.consume(available);
            }
            if eof {
                break true;
            }
            if available == 0 && transfer.is_done() {
                break false;
            }
            core::hint::spin_loop();
        };

        if synced {
            self.stream = Some(Stream::Running(transfer));
            Ok(())
        } else {
            self.stream = Some(Stream::Running(transfer).stopped());
            Err(CaptureError::StreamEnded)
        }
    }

    /// Copy every byte DMA has delivered into the capture buffer, up to the
    /// next VSYNC. Never waits for more data.
    fn pump(&mut self) {
        let Progress::Filling(mut bytes) = self.progress else {
            return;
        };

        if self.is_stopped() && (bytes != 0 || self.start_stream_aligned().is_err()) {
            self.progress = Progress::Complete(bytes);
            return;
        }
        let Some(Stream::Running(transfer)) = self.stream.as_mut() else {
            unreachable!("stream is running after start_stream_aligned");
        };

        loop {
            let (chunk, eof) = transfer.peek_until_eof();
            let available = chunk.len();
            let copy_len = available.min(FRAME_BYTES.saturating_sub(bytes));
            self.capture_buffer[bytes..bytes + copy_len].copy_from_slice(&chunk[..copy_len]);
            bytes = bytes.saturating_add(available);
            if available != 0 {
                transfer.consume(available);
            }
            if eof {
                self.progress = Progress::Complete(bytes);
                break;
            }
            if available == 0 {
                self.progress = Progress::Filling(bytes);
                break;
            }
        }

        if transfer.is_done() {
            // DMA ended without VSYNC: the frame cannot complete.
            if let Progress::Filling(bytes) = self.progress {
                self.progress = Progress::Complete(bytes);
            }
            self.stop_stream();
        }
    }

    /// Wait for the rest of the frame in progress, check its size and swap it
    /// into the display buffer.
    fn finish_capture(&mut self) -> Result<(), CaptureError> {
        while let Progress::Filling(_) = self.progress {
            self.pump();
            if self.is_stopped() {
                break;
            }
            core::hint::spin_loop();
        }

        let result = match self.progress {
            Progress::Complete(FRAME_BYTES) => {
                core::mem::swap(&mut self.display_buffer, &mut self.capture_buffer);
                self.display_ready = true;
                Ok(())
            }
            Progress::Complete(bytes) | Progress::Filling(bytes) => {
                self.display_ready = false;
                Err(CaptureError::BadLength(bytes))
            }
        };
        self.progress = Progress::Filling(0);
        result
    }

    /// Capture one whole frame while nothing is shown yet.
    fn prime(&mut self) -> Result<(), CaptureError> {
        self.display_ready = false;
        self.progress = Progress::Filling(0);
        self.start_stream_aligned()?;
        self.finish_capture()
    }

    fn report_bad_frame(&mut self, error: CaptureError) {
        self.bad_frames = self.bad_frames.saturating_add(1);
        if self.bad_frames == 1 || self.bad_frames.is_multiple_of(BAD_FRAME_LOG_INTERVAL) {
            warn!(
                "Camera frame dropped: {} ({} bad frames so far)",
                error, self.bad_frames
            );
        }
    }
}
