//! The camera capture pipeline on CPU0.
//!
//! The GC0308 sends 320x240 RGB565 over an 8-bit parallel bus. The
//! `LCD_CAM` peripheral streams those bytes into a small DMA ring buffer.
//! Three PSRAM frame buffers decouple the sensor's timing from the display's:
//! one complete frame is shown while CPU0 copies the next one out of the
//! ring, during the milliseconds the display's own DMA transfer is busy. The
//! sensor's VSYNC signal marks where one frame ends and the next begins.
//!
//! ```text
//! sensor ──> DMA ring ──(while the LCD is busy)──> capture buffer
//!                                                     │ complete at VSYNC
//!                                                  ready buffer
//!                                                     │ swap in `finish`
//!                                  LCD <── display buffer
//! ```
//!
//! The sensor never pauses, and drawing a frame takes longer than the sensor
//! needs to send one. A frame therefore often completes while the previous
//! one is still being drawn; it waits in the ready buffer, and capture goes
//! on into the capture buffer, so the ring is always being drained. With
//! only two buffers, the ring would back up behind the finished frame,
//! overflow and stop the DMA.

use embassy_time::{Duration, Instant};
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
/// How long to wait for a frame boundary before giving up: several frame
/// periods, so a sensor that answers on I2C but sends no frames cannot hang
/// the application.
const FRAME_TIMEOUT: Duration = Duration::from_millis(250);

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
    /// No transfer running: after `init`, after `pause`, after the ring
    /// overflowed and while restarting.
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

/// Why a frame could not be captured.
enum CaptureError {
    /// The DMA transfer did not start.
    DmaStart,
    /// Nothing drained the DMA ring for too long: it overflowed and the
    /// transfer stopped before the sensor signalled VSYNC.
    StreamEnded,
    /// The sensor signalled no frame boundary within `FRAME_TIMEOUT`.
    Timeout,
    /// VSYNC came after this many bytes instead of a whole frame.
    BadLength(usize),
}

impl core::fmt::Display for CaptureError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::DmaStart => write!(f, "DMA start failed"),
            Self::StreamEnded => write!(f, "the DMA ring overflowed before VSYNC"),
            Self::Timeout => write!(f, "no VSYNC within {} ms", FRAME_TIMEOUT.as_millis()),
            Self::BadLength(bytes) => write!(f, "{bytes} of {FRAME_BYTES} bytes before VSYNC"),
        }
    }
}

/// Application handle for the camera; see the [module docs](super).
///
/// Show frames with [`Camera::begin_frame`], draw the [`Frame`] and call
/// [`Frame::finish`]. Call [`Camera::pump`] wherever the loop does other work
/// between frames, and [`Camera::pause`] when the preview is hidden, so the
/// next frame starts cleanly.
pub struct Camera {
    /// The camera driver and ring buffer, in whichever state they are.
    // `None` only while a method moves the stream between states.
    stream: Option<Stream>,
    /// The complete frame being shown, in PSRAM.
    display_buffer: &'static mut [u8],
    /// The frame being filled from the DMA ring, in PSRAM.
    capture_buffer: &'static mut [u8],
    /// The newest complete frame not shown yet, in PSRAM. Capture goes on
    /// into `capture_buffer` while this one waits, so the ring never backs
    /// up behind a finished frame.
    ready_buffer: &'static mut [u8],
    /// Whether `ready_buffer` holds a frame newer than `display_buffer`.
    ready: bool,
    /// Whether `display_buffer` holds a whole frame.
    display_ready: bool,
    /// Bytes of the frame in progress copied into `capture_buffer` so far.
    filled: usize,
    /// A frame that ended at VSYNC with this many bytes instead of a whole
    /// frame; reported once the next good frame is delivered.
    short_frame: Option<usize>,
    /// Frames dropped so far, for rate-limited logging.
    bad_frames: u32,
}

/// One frozen camera frame being shown while the following frames are captured.
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

    /// Make the next complete frame the frame to show.
    ///
    /// Returns at once if a frame completed while this one was drawn, and
    /// otherwise waits for the sensor's next whole frame: normally one frame
    /// period, two if the frame in progress turns out short. A capture fault
    /// is logged and the following `begin_frame` re-syncs.
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

/// Configure `LCD_CAM` for the sensor and allocate the three frame buffers.
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
        ready_buffer: storage::leaked_slice(FRAME_BYTES, 0),
        ready: false,
        display_ready: false,
        filled: 0,
        short_frame: None,
        bad_frames: 0,
    }
}

impl Camera {
    /// Stop capturing. The next `begin_frame` re-syncs to a fresh VSYNC
    /// instead of showing a stale partial frame.
    pub fn pause(&mut self) {
        self.stop_stream();
        self.display_ready = false;
        self.ready = false;
        self.filled = 0;
        self.short_frame = None;
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
    /// to capture a complete frame. `None`, with a logged warning, when the
    /// sensor delivers nothing within a quarter of a second or the DMA fails.
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

        let deadline = Instant::now() + FRAME_TIMEOUT;
        let synced = loop {
            let (chunk, eof) = transfer.peek_until_eof();
            let available = chunk.len();
            if available != 0 {
                transfer.consume(available);
            }
            if eof {
                break Ok(());
            }
            if available == 0 && transfer.is_done() {
                break Err(CaptureError::StreamEnded);
            }
            if Instant::now() > deadline {
                break Err(CaptureError::Timeout);
            }
            core::hint::spin_loop();
        };

        if synced.is_ok() {
            self.stream = Some(Stream::Running(transfer));
        } else {
            self.stream = Some(Stream::Running(transfer).stopped());
        }
        synced
    }

    /// Copy every byte DMA has delivered into the frame in progress and, past
    /// its VSYNC, into the following frame. Never waits for more data; does
    /// nothing while the stream is stopped.
    fn pump_capture(&mut self) {
        let Some(Stream::Running(transfer)) = self.stream.as_mut() else {
            return;
        };

        loop {
            let (chunk, eof) = transfer.peek_until_eof();
            let available = chunk.len();
            if available == 0 && !eof {
                break;
            }
            // Copy one descriptor at a time and hand it straight back to the
            // DMA. The copy into PSRAM is slow, and returning a whole run of
            // descriptors only after copying it all would starve the DMA
            // meanwhile.
            let take = available.min(STREAM_CHUNK_BYTES);
            let copy_len = take.min(FRAME_BYTES.saturating_sub(self.filled));
            // Past the end of the buffer (a missed VSYNC) bytes are only
            // counted, so the frame is reported as too long and dropped.
            if copy_len != 0 {
                self.capture_buffer[self.filled..self.filled + copy_len]
                    .copy_from_slice(&chunk[..copy_len]);
            }
            self.filled = self.filled.saturating_add(take);
            // Consuming zero bytes still returns an empty descriptor that
            // carries only the VSYNC flag.
            transfer.consume(take);
            if eof && take == available {
                // The frame ended. A whole frame moves to the ready buffer;
                // anything else is dropped. Either way capture continues.
                if self.filled == FRAME_BYTES {
                    core::mem::swap(&mut self.capture_buffer, &mut self.ready_buffer);
                    self.ready = true;
                } else {
                    self.short_frame = Some(self.filled);
                }
                self.filled = 0;
            }
        }

        if transfer.is_done() {
            // The ring overflowed and the DMA stopped; `finish_capture`
            // reports it and `begin_frame` starts afresh.
            self.stop_stream();
        }
    }

    /// Wait until a whole frame is ready and make it the frame to show.
    fn finish_capture(&mut self) -> Result<(), CaptureError> {
        let deadline = Instant::now() + FRAME_TIMEOUT;
        loop {
            if self.ready {
                self.ready = false;
                core::mem::swap(&mut self.display_buffer, &mut self.ready_buffer);
                self.display_ready = true;
                if let Some(bytes) = self.short_frame.take() {
                    // Logging blocks for milliseconds while the sensor keeps
                    // streaming, so this can cost the frame in progress; the
                    // rate limit keeps that rare.
                    self.report_bad_frame(CaptureError::BadLength(bytes));
                }
                return Ok(());
            }
            let failure = if self.is_stopped() {
                Some(CaptureError::StreamEnded)
            } else if Instant::now() > deadline {
                self.stop_stream();
                Some(CaptureError::Timeout)
            } else {
                None
            };
            if let Some(error) = failure {
                self.display_ready = false;
                self.filled = 0;
                self.short_frame = None;
                return Err(error);
            }
            self.pump_capture();
            core::hint::spin_loop();
        }
    }

    /// Capture one whole frame while nothing is shown yet.
    fn prime(&mut self) -> Result<(), CaptureError> {
        self.display_ready = false;
        self.ready = false;
        self.filled = 0;
        self.short_frame = None;
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
