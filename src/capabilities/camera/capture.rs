//! The camera capture pipeline on CPU0.
//!
//! The GC0308 sends 320x240 RGB565 over an 8-bit parallel bus. The
//! `LCD_CAM` peripheral streams those bytes into a small DMA ring buffer.
//! Two PSRAM frame buffers decouple the sensor's timing from the display's:
//! one complete frame is shown while CPU0 copies the next one out of the
//! ring, during the milliseconds the display's own DMA transfer is busy. The
//! sensor's VSYNC signal marks where one frame ends and the next begins.
//!
//! ```text
//! sensor ──> DMA ring ──(while the LCD is busy)──> capture buffer
//!                                                     │ swap on VSYNC
//!                                  LCD <── display buffer
//! ```

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

/// Width of a camera frame in pixels.
pub const WIDTH: usize = 320;
/// Height of a camera frame in pixels.
pub const HEIGHT: usize = 240;
/// RGB565, like the display.
const BYTES_PER_PIXEL: usize = crate::capabilities::display::BYTES_PER_PIXEL;
/// Bytes in one row of a frame.
const SCANLINE_BYTES: usize = WIDTH * BYTES_PER_PIXEL;
/// Bytes in one whole frame.
const FRAME_BYTES: usize = WIDTH * HEIGHT * BYTES_PER_PIXEL;

/// One DMA descriptor of the ring: five rows.
///
/// The ring lives in internal RAM, which is scarce, so it holds only a sixth
/// of a frame: 40 rows, a few milliseconds of sensor output. The sensor never
/// stops, so someone must drain the ring at least that often: the renderer
/// does it while each LCD DMA batch is in flight, and [`Camera::pump`] does
/// it between frames. When the ring fills, the DMA stops and the frame is
/// dropped.
///
/// Do not enlarge it casually: every static byte of internal RAM comes out
/// of the main stack of CPU0, which is what is left of DRAM after the
/// statics. Doubling the ring left about 20 KiB of stack and the demo
/// overflowed it while building its screens.
const STREAM_CHUNK_BYTES: usize = SCANLINE_BYTES * 5;
/// The whole DMA ring.
const STREAM_BUFFER_BYTES: usize = STREAM_CHUNK_BYTES * 8;
/// Log only the first bad frame and then every 32nd, not all of them.
const BAD_FRAME_LOG_INTERVAL: u32 = 32;

const _: () = assert!(STREAM_CHUNK_BYTES <= esp_hal::dma::CHUNK_SIZE);
const _: () = assert!(STREAM_BUFFER_BYTES.is_multiple_of(STREAM_CHUNK_BYTES));

/// A DMA transfer that is streaming camera bytes into the ring buffer. It
/// owns the driver and the buffer until it is stopped.
type InFlight = CameraTransfer<'static, DmaRxStreamBuf>;

/// The peripherals and pins wired to the camera sensor, consumed once by
/// [`init`].
pub(crate) struct Resources {
    /// The camera interface peripheral that samples the parallel bus.
    pub(crate) lcd_cam: LCD_CAM<'static>,
    /// The DMA channel that moves captured bytes into memory without the CPU.
    pub(crate) dma: DMA_CH2<'static>,
    /// Pixel clock (PCLK): the sensor presents one data byte per clock edge.
    pub(crate) pclk: GPIO45<'static>,
    /// Frame sync (VSYNC): marks the boundary between two frames.
    pub(crate) vsync: GPIO46<'static>,
    /// Line valid (HREF): high while a row's pixel bytes are on the bus.
    pub(crate) href: GPIO38<'static>,
    /// Data bit 0 (least significant) of the parallel bus.
    pub(crate) d0: GPIO39<'static>,
    /// Data bit 1 of the parallel bus.
    pub(crate) d1: GPIO40<'static>,
    /// Data bit 2 of the parallel bus.
    pub(crate) d2: GPIO41<'static>,
    /// Data bit 3 of the parallel bus.
    pub(crate) d3: GPIO42<'static>,
    /// Data bit 4 of the parallel bus.
    pub(crate) d4: GPIO15<'static>,
    /// Data bit 5 of the parallel bus.
    pub(crate) d5: GPIO16<'static>,
    /// Data bit 6 of the parallel bus.
    pub(crate) d6: GPIO48<'static>,
    /// Data bit 7 (most significant) of the parallel bus.
    pub(crate) d7: GPIO47<'static>,
}

/// The camera driver and its ring buffer, either idle or streaming.
enum Stream {
    /// No transfer running: after `init`, after `pause` and while restarting.
    Stopped {
        /// The configured camera driver, ready to start a transfer.
        driver: CameraDriver<'static>,
        /// The DMA ring buffer, reused by the next transfer.
        buffer: DmaRxStreamBuf,
    },
    /// A transfer is streaming; bytes accumulate in the ring until read.
    Running(InFlight),
}

impl Stream {
    /// The same stream, stopped if it was running.
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

/// Why a frame could not be captured.
enum CaptureError {
    /// The DMA transfer did not start.
    DmaStart,
    /// The DMA transfer stopped before the sensor signalled VSYNC.
    StreamEnded,
    /// VSYNC came after this many bytes instead of a whole frame.
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

/// Application handle for the camera; see the [module docs](super).
///
/// Show frames with [`Camera::begin_frame`], draw the [`Frame`] and call
/// [`Frame::finish`]. Call [`Camera::pause`] when the preview is hidden, so
/// the next frame starts cleanly.
pub struct Camera {
    /// The camera driver and ring buffer, in whichever state they are.
    // `None` only while a method moves the stream between states.
    stream: Option<Stream>,
    /// The complete frame being shown, in PSRAM.
    display_buffer: &'static mut [u8],
    /// The next frame, filled from the DMA ring; swapped with
    /// `display_buffer` when complete.
    capture_buffer: &'static mut [u8],
    /// Whether `display_buffer` holds a whole frame.
    display_ready: bool,
    /// How far `capture_buffer` is filled.
    progress: Progress,
    /// Frames dropped so far, for rate-limited logging.
    bad_frames: u32,
}

/// One frozen camera frame being shown while the following frame is captured.
pub struct Frame<'a> {
    /// The camera whose display buffer is shown and whose capture buffer
    /// keeps filling.
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
        self.camera.pump_capture();
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

/// Configure `LCD_CAM` for the sensor and allocate the two frame buffers.
/// Capturing starts with the first [`Camera::begin_frame`].
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
        display_buffer: storage::leaked_slice(FRAME_BYTES, 0),
        capture_buffer: storage::leaked_slice(FRAME_BYTES, 0),
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

    /// Copy whatever the sensor has delivered of the next frame, without
    /// waiting. Does nothing while the camera is paused.
    ///
    /// The sensor streams continuously into a small buffer that holds only a
    /// few milliseconds of data. Drawing a [`Frame`] drains it, but between
    /// `finish` and the next `begin_frame` nothing does, so a loop that sleeps
    /// or does other work between frames should call this often, for example
    /// once per iteration. Otherwise frames are dropped.
    pub fn pump(&mut self) {
        if self.display_ready {
            self.pump_capture();
        }
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

    /// Stop the DMA transfer if it is running.
    fn stop_stream(&mut self) {
        self.stream = self.stream.take().map(Stream::stopped);
    }

    /// Whether the DMA transfer is stopped.
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
    /// next VSYNC. Never waits for more data. Starts the stream if it is
    /// stopped, which blocks until the next VSYNC.
    fn pump_capture(&mut self) {
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
            self.pump_capture();
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

    /// Count a dropped frame and log it, rate-limited.
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
