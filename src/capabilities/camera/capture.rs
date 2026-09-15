//! The camera capture pipeline on CPU0.
//!
//! The GC0308 camera sensor sends 320x240 pixels in RGB565 over an 8-bit
//! parallel bus. The `LCD_CAM` peripheral of the ESP32-S3 receives these
//! bytes. DMA (direct memory access) copies them into a small ring buffer,
//! the DMA ring, without the CPU.
//!
//! Three frame buffers in PSRAM (the external RAM chip) separate the
//! sensor's timing from the display's timing. One complete frame is shown.
//! At the same time, CPU0 copies the next frame out of the ring. It does
//! this in the milliseconds while the display's own DMA transfer is busy.
//! The sensor's VSYNC (vertical sync) signal marks where one frame ends and
//! the next begins.
//!
//! ```text
//! sensor ──> DMA ring ──(while the LCD is busy)──> capture buffer
//!                                                     │ complete at VSYNC
//!                                                  ready buffer
//!                                                     │ swap in `finish`
//!                                  LCD <── display buffer
//! ```
//!
//! The sensor never pauses. Drawing a frame takes longer than the sensor
//! needs to send one. So a frame often completes while the previous frame is
//! still being drawn. The completed frame waits in the ready buffer, and
//! capture continues into the capture buffer. In this way, the CPU empties
//! the ring all the time. With only two buffers, capture would have to wait
//! for the display to take the completed frame. The ring would then
//! overflow, and the DMA transfer would stop.

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

use crate::{board::psram, capabilities::display::ScanlineSource};

/// Width of a camera frame in pixels.
pub const WIDTH: usize = 320;
/// Height of a camera frame in pixels.
pub const HEIGHT: usize = 240;
/// Bytes per pixel: 2, because the camera sends RGB565, like the display.
const BYTES_PER_PIXEL: usize = crate::capabilities::display::BYTES_PER_PIXEL;
/// Bytes in one row of a frame: 640.
const SCANLINE_BYTES: usize = WIDTH * BYTES_PER_PIXEL;
/// Bytes in one whole frame: 153,600.
const FRAME_BYTES: usize = WIDTH * HEIGHT * BYTES_PER_PIXEL;

/// One DMA descriptor of the ring: five rows (3,200 bytes).
///
/// A DMA descriptor is one block of the ring. The DMA fills the blocks one
/// after the other.
///
/// The ring is in internal RAM, and there is little internal RAM. So the ring
/// holds only a sixth of a frame: 40 rows, a few milliseconds of sensor data.
/// The sensor never stops. So code must empty the ring at least that often.
/// [`Surface::render_from`](crate::capabilities::display::Surface::render_from)
/// does it while the LCD DMA sends each batch of rows. [`Camera::pump`] does
/// it between frames. When the ring is full, the DMA transfer stops and the
/// frame is dropped.
///
/// Do not make the ring larger without a good reason. Every static byte in
/// internal RAM makes the main stack of CPU0 smaller: that stack gets the
/// part of DRAM (data RAM) that the statics do not use. With a ring twice
/// this size, only about 20 KiB of stack was left, and the demo overflowed
/// its stack while it built its screens.
const STREAM_CHUNK_BYTES: usize = SCANLINE_BYTES * 5;
/// The whole DMA ring: eight descriptors, 40 rows (25,600 bytes).
const STREAM_BUFFER_BYTES: usize = STREAM_CHUNK_BYTES * 8;
/// Log the first bad frame and then only every 32nd bad frame.
const BAD_FRAME_LOG_INTERVAL: u32 = 32;
/// How long one wait for a frame boundary or a whole frame may take. This is
/// several frame periods. With this limit, a sensor that answers on I2C but
/// sends no frames cannot block the application forever.
const FRAME_TIMEOUT: Duration = Duration::from_millis(250);

// Compile-time checks: one descriptor may hold at most esp-hal's
// `CHUNK_SIZE` bytes, and the ring must consist of whole descriptors.
const _: () = assert!(STREAM_CHUNK_BYTES <= esp_hal::dma::CHUNK_SIZE);
const _: () = assert!(STREAM_BUFFER_BYTES.is_multiple_of(STREAM_CHUNK_BYTES));

/// A running DMA transfer that copies camera bytes into the DMA ring. It
/// owns the driver and the ring until it stops.
type InFlight = CameraTransfer<'static, DmaRxStreamBuf>;

/// The peripherals and pins connected to the camera sensor. [`init`] takes
/// them once.
pub(crate) struct Resources {
    /// The camera interface peripheral (`LCD_CAM`). It reads the parallel
    /// bus.
    pub(crate) lcd_cam: LCD_CAM<'static>,
    /// The DMA channel that copies received bytes into memory without the
    /// CPU.
    pub(crate) dma: DMA_CH2<'static>,
    /// Pixel clock (PCLK): the sensor puts one data byte on the bus for each
    /// clock cycle.
    pub(crate) pclk: GPIO45<'static>,
    /// Frame sync (VSYNC): marks the boundary between two frames.
    pub(crate) vsync: GPIO46<'static>,
    /// Line valid (HREF, horizontal reference): high while the bytes of a row
    /// are on the bus.
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

/// The camera driver and its DMA ring, either stopped or running.
enum Stream {
    /// No transfer runs: after `init`, after `pause`, after the ring
    /// overflowed, after a timeout, and during a restart.
    Stopped {
        /// The configured camera driver, ready to start a transfer.
        driver: CameraDriver<'static>,
        /// The DMA ring. The next transfer uses it again.
        buffer: DmaRxStreamBuf,
    },
    /// A transfer runs. Bytes collect in the ring until the CPU reads them.
    Running(InFlight),
}

impl Stream {
    /// The same stream, stopped. Stop the transfer first if it runs.
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
    /// Nothing emptied the DMA ring for too long. The ring overflowed, and
    /// the transfer stopped during the wait for VSYNC or for a whole frame.
    StreamEnded,
    /// The wait ended after [`FRAME_TIMEOUT`]. Either the sensor sent no
    /// frame boundary (VSYNC), or no whole frame arrived. Frames with the
    /// wrong length do not count as whole frames.
    Timeout,
    /// VSYNC came after this many bytes instead of after exactly
    /// [`FRAME_BYTES`]. The number can be smaller or larger.
    BadLength(usize),
}

impl core::fmt::Display for CaptureError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::DmaStart => write!(f, "DMA start failed"),
            Self::StreamEnded => write!(f, "the DMA ring overflowed before a whole frame arrived"),
            Self::Timeout => write!(f, "no whole frame within {} ms", FRAME_TIMEOUT.as_millis()),
            Self::BadLength(bytes) => write!(f, "{bytes} of {FRAME_BYTES} bytes before VSYNC"),
        }
    }
}

/// Application handle for the camera; see the [module docs](super).
///
/// To show frames, call [`Camera::begin_frame`], draw the [`Frame`], then
/// call [`Frame::finish`]. When the loop does other work between frames,
/// call [`Camera::pump`] there. When the camera image is not shown, call
/// [`Camera::pause`], so the next frame starts cleanly.
pub struct Camera {
    /// The camera driver and the DMA ring, in their current state.
    // `None` only while a method moves the stream from one state to the other.
    stream: Option<Stream>,
    /// The display buffer: the complete frame that is shown, in PSRAM.
    display_buffer: &'static mut [u8],
    /// The capture buffer: the frame that the CPU fills from the DMA ring,
    /// in PSRAM.
    capture_buffer: &'static mut [u8],
    /// The ready buffer: the newest complete frame that is not shown yet, in
    /// PSRAM. While a frame waits here, capture continues into
    /// `capture_buffer`. So the CPU can always empty the ring.
    ready_buffer: &'static mut [u8],
    /// Whether `ready_buffer` holds a frame newer than `display_buffer`.
    ready: bool,
    /// Whether `display_buffer` holds a whole frame.
    display_ready: bool,
    /// How many bytes of the frame in progress arrived so far. Bytes past
    /// [`FRAME_BYTES`] are counted but not copied into `capture_buffer`.
    filled: usize,
    /// The byte count of the last frame that ended at VSYNC with the wrong
    /// length. The name says "short", but the frame can also be too long.
    /// It is reported when the next whole frame is shown.
    short_frame: Option<usize>,
    /// Frames dropped so far. Used to log only some of them.
    bad_frames: u32,
}

/// One complete camera frame to draw. It does not change while you draw it,
/// and the camera captures the next frame in the meantime.
pub struct Frame<'a> {
    /// The camera. Its display buffer holds this frame, and its capture
    /// buffer continues to fill.
    camera: &'a mut Camera,
}

impl Frame<'_> {
    /// The big-endian RGB565 bytes of row `y`: [`WIDTH`] pixels of 2 bytes
    /// each.
    ///
    /// # Panics
    ///
    /// When `y` is [`HEIGHT`] or more.
    pub fn scanline(&self, y: usize) -> &[u8] {
        debug_assert!(y < HEIGHT);
        let start = y * SCANLINE_BYTES;
        &self.camera.display_buffer[start..start + SCANLINE_BYTES]
    }

    /// Copy the data that the sensor has sent so far for the next frame.
    /// Never waits.
    ///
    /// [`Surface::render_from`](crate::capabilities::display::Surface::render_from)
    /// already calls this while the display is busy. Call it yourself when
    /// you draw the frame in another way and the CPU would otherwise wait.
    pub fn pump(&mut self) {
        self.camera.pump_capture();
    }

    /// Make the next complete frame the frame to show, and end this `Frame`.
    ///
    /// Return at once when a frame was completed while this one was drawn.
    /// Otherwise, wait for the next complete frame from the sensor: normally
    /// one frame period, two when the frame in progress is too short. Wait at
    /// most a quarter of a second. When the capture fails, log a warning; the
    /// next [`Camera::begin_frame`] then starts the capture again.
    pub fn finish(self) {
        if let Err(error) = self.camera.finish_capture() {
            self.camera.report_bad_frame(error);
        }
    }
}

/// Drawing a `Frame` with
/// [`Surface::render_from`](crate::capabilities::display::Surface::render_from)
/// shows the top-left part of the image when the surface is smaller than
/// 320x240.
impl ScanlineSource for Frame<'_> {
    fn fill_row(&mut self, y: usize, row: &mut [u8]) {
        row.copy_from_slice(&self.scanline(y)[..row.len()]);
    }

    fn while_transferring(&mut self) {
        self.pump();
    }
}

/// Configure `LCD_CAM` for the sensor, create the DMA ring and allocate the
/// three frame buffers in PSRAM. Capturing starts with the first
/// [`Camera::begin_frame`].
///
/// # Panics
///
/// When esp-hal rejects the camera configuration, or PSRAM has no room for
/// the frame buffers.
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
    // The board has its own 20 MHz clock for the sensor, and there is no XCLK
    // (external clock) pin from the ESP32-S3 to the sensor. So LCD_CAM runs in
    // slave mode: the driver gets no master clock pin, and no pin outputs the
    // 20 MHz clock that this configuration sets. The default configuration
    // also sets the end-of-frame (EOF) mode to VSYNC: at each VSYNC, the DMA
    // marks the end of a frame in the ring, so the CPU can find frame
    // boundaries.
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
        display_buffer: psram::leaked_slice(FRAME_BYTES, 0),
        capture_buffer: psram::leaked_slice(FRAME_BYTES, 0),
        ready_buffer: psram::leaked_slice(FRAME_BYTES, 0),
        ready: false,
        display_ready: false,
        filled: 0,
        short_frame: None,
        bad_frames: 0,
    }
}

impl Camera {
    /// Stop capturing. The next [`Camera::begin_frame`] waits for the start
    /// of a new frame, so it does not show an old or incomplete frame.
    pub fn pause(&mut self) {
        self.stop_stream();
        self.display_ready = false;
        self.ready = false;
        self.filled = 0;
        self.short_frame = None;
    }

    /// Copy the data that the sensor has sent so far for the next frame.
    /// Never waits. Does nothing while the camera is paused, and before the
    /// first [`Camera::begin_frame`].
    ///
    /// The sensor sends data all the time into a small buffer. The buffer
    /// holds only a few milliseconds of data. Drawing a [`Frame`] empties the
    /// buffer, but between `finish` and the next `begin_frame`, nothing does.
    /// So a loop that sleeps or does other work between frames should call
    /// this often, for example once per loop iteration. Otherwise frames are
    /// dropped.
    pub fn pump(&mut self) {
        if self.display_ready {
            self.pump_capture();
        }
    }

    /// Get the current frame to draw.
    ///
    /// The first call after start-up or after [`Camera::pause`] waits for a
    /// complete frame, normally up to two frame periods. Later calls return
    /// at once.
    ///
    /// Return `None` and log a warning when the capture fails: the DMA
    /// transfer does not start, the DMA buffer overflows, or the sensor sends
    /// no frame in time (each wait ends after a quarter of a second). The
    /// next call tries again.
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

    /// Restart the DMA transfer and wait for the next VSYNC. Throw away the
    /// bytes before it, so the next unread byte is the first byte of a new
    /// frame.
    ///
    /// # Errors
    ///
    /// [`CaptureError::DmaStart`] when the transfer does not start,
    /// [`CaptureError::StreamEnded`] when the ring overflows first, and
    /// [`CaptureError::Timeout`] when no VSYNC comes within [`FRAME_TIMEOUT`].
    /// After an error, the stream is stopped.
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

    /// Copy every byte that the DMA has received into the frame in progress.
    /// Bytes after its VSYNC go into the next frame. Never waits for more
    /// data. Does nothing while the stream is stopped.
    ///
    /// A whole frame goes to the ready buffer and replaces an older frame
    /// there that was not shown.
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
            // Copy at most one descriptor, then give it back to the DMA at
            // once. The copy into PSRAM is slow. If the CPU copied many
            // descriptors before it gave them back, the DMA would have no free
            // descriptors during that time.
            let take = available.min(STREAM_CHUNK_BYTES);
            let copy_len = take.min(FRAME_BYTES.saturating_sub(self.filled));
            // After a missed VSYNC, the frame is longer than the buffer. Such
            // bytes are only counted, so the frame is reported as too long and
            // dropped.
            if copy_len != 0 {
                self.capture_buffer[self.filled..self.filled + copy_len]
                    .copy_from_slice(&chunk[..copy_len]);
            }
            self.filled = self.filled.saturating_add(take);
            // `consume(0)` is still necessary: it gives an empty descriptor
            // back to the DMA. Such a descriptor carries only the VSYNC flag.
            transfer.consume(take);
            if eof && take == available {
                // The frame ended. A whole frame moves to the ready buffer.
                // A frame with the wrong length is dropped. In both cases,
                // capture continues with the next frame.
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
            // The ring overflowed and the DMA transfer stopped.
            // `finish_capture` reports it, and the next `begin_frame` starts
            // a new capture.
            self.stop_stream();
        }
    }

    /// Wait until a whole frame is ready, and make it the frame to show.
    ///
    /// When a frame with the wrong length came before it, log that frame now.
    ///
    /// # Errors
    ///
    /// [`CaptureError::StreamEnded`] when the stream is stopped, for example
    /// after the ring overflowed. [`CaptureError::Timeout`] when no whole
    /// frame is ready within [`FRAME_TIMEOUT`]; the stream is then stopped.
    /// After an error, nothing is shown, so the next
    /// [`Camera::begin_frame`] starts a new capture.
    fn finish_capture(&mut self) -> Result<(), CaptureError> {
        let deadline = Instant::now() + FRAME_TIMEOUT;
        loop {
            if self.ready {
                self.ready = false;
                core::mem::swap(&mut self.display_buffer, &mut self.ready_buffer);
                self.display_ready = true;
                if let Some(bytes) = self.short_frame.take() {
                    // Logging blocks for milliseconds, and the sensor
                    // continues to send data during that time. So the log
                    // message can cost the frame in progress. Only some bad
                    // frames are logged, so this happens rarely.
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

    /// Capture the first whole frame when nothing is shown yet: after
    /// start-up, after `pause`, or after an error.
    ///
    /// Waits for the next VSYNC and then for one whole frame. Each wait
    /// takes at most [`FRAME_TIMEOUT`].
    ///
    /// # Errors
    ///
    /// The errors of [`Camera::start_stream_aligned`] and
    /// [`Camera::finish_capture`].
    fn prime(&mut self) -> Result<(), CaptureError> {
        self.display_ready = false;
        self.ready = false;
        self.filled = 0;
        self.short_frame = None;
        self.start_stream_aligned()?;
        self.finish_capture()
    }

    /// Count a dropped frame. Log a warning for the first dropped frame and
    /// then for every [`BAD_FRAME_LOG_INTERVAL`]th one.
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
