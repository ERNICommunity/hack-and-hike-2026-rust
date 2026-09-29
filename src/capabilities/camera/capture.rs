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
//!                                                     │ or `advance`
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
//!
//! # Frames on demand
//!
//! Copying a frame out of the ring costs CPU0 about 10 ms, and the writes to
//! PSRAM push other data out of the cache. An application that shows ten
//! frames per second and computes between them does not want every frame
//! the sensor sends. After [`Camera::capture_on_demand`], the camera copies
//! a frame only when [`Camera::request_frame`] asked for one before the
//! frame began. The bytes of the other frames leave the ring without a
//! copy, which costs almost nothing.
//!
//! # Handing a frame over
//!
//! [`Frame::take`] gives the frame's buffer to the application and takes a
//! spare buffer in exchange: a frame for a long computation, without a copy
//! of its 150 KiB.
//!
//! # Without waiting
//!
//! [`Camera::begin_frame`] and [`Frame::finish`] wait for the sensor when
//! they have to: at the start, and after the ring overflowed, for up to two
//! frame periods (100 ms), or half a second when the sensor sends nothing. That is right for a loop that only shows the
//! camera. It is wrong for a task on an interrupt executor: while it
//! waits, the task it interrupted stands still, and so does the timer of
//! both CPU cores, which has the same interrupt priority.
//!
//! Such a task uses three other functions, which never wait:
//!
//! - [`Camera::service`] empties the ring. When the capture is stopped (at
//!   the start, after an overflow), it starts it again, and the bytes up to
//!   the next frame boundary are thrown away as they come.
//! - [`Camera::advance`] makes the newest whole frame the current one, when
//!   there is a newer one.
//! - [`Camera::current`] is the current frame, when there is one.
//!
//! An overflow then costs the frames that were on their way, and no time.
//! Use either these three or `begin_frame` and `finish`, not both.

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
/// it between frames, and [`Camera::service`] does it for code that never
/// waits. When the ring is full, the DMA transfer stops and the frame is
/// dropped.
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
///
/// Code that must never wait uses [`Camera::service`], [`Camera::advance`]
/// and [`Camera::current`] instead, and perhaps
/// [`Camera::capture_on_demand`] with [`Camera::request_frame`]; see the
/// [module docs](super). Do not mix the two ways.
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
    /// Whether frames are copied only on request; see
    /// [`Camera::capture_on_demand`].
    on_demand: bool,
    /// Whether the next frame that begins is wanted: set by
    /// [`Camera::request_frame`], used up when that frame begins.
    wanted: bool,
    /// Whether the frame in progress is copied into `capture_buffer`. When
    /// not, its bytes are only counted.
    copying: bool,
    /// Whether the stream is between a start without waiting
    /// ([`Camera::service`]) and the first frame boundary after it: the
    /// bytes are the rest of a frame, and are thrown away.
    aligning: bool,
    /// Whether `display_buffer` holds a frame that was not handed over
    /// with [`Frame::take`].
    display_whole: bool,
    /// Whether the ring overflowed and [`Camera::service`] has not dealt
    /// with it yet.
    overflowed: bool,
    /// When the ring was emptied last, while the stream runs.
    last_pump: Option<Instant>,
    /// What the capture did since the last [`Camera::take_stats`].
    stats: CaptureStats,
    /// The byte count of the last frame that ended at VSYNC with the wrong
    /// length. The name says "short", but the frame can also be too long.
    /// It is reported when the next whole frame is shown.
    short_frame: Option<usize>,
    /// Frames dropped so far. Used to log only some of them.
    bad_frames: u32,
}

/// What the capture did in a stretch of time; see [`Camera::take_stats`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaptureStats {
    /// The longest time between two times the ring was emptied. The ring
    /// overflows when this comes near the time it holds, about 5 ms.
    pub longest_gap: Duration,
    /// The time spent copying frames out of the ring into PSRAM.
    pub copy_time: Duration,
    /// The whole frames copied.
    pub frames_copied: u32,
}

impl CaptureStats {
    /// Nothing done yet.
    pub const NONE: Self = Self {
        longest_gap: Duration::from_ticks(0),
        copy_time: Duration::from_ticks(0),
        frames_copied: 0,
    };
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

    /// Whether [`Frame::finish`] would return at once: a newer whole frame
    /// is waiting, or the capture stopped because the DMA ring overflowed.
    /// In the second case `finish` reports the dropped frame, and the next
    /// [`Camera::begin_frame`] starts the capture again. Call
    /// [`Camera::pump`] before, so the check sees the newest data.
    ///
    /// A loop that calls `finish` only when this is true never waits for the
    /// sensor and still recovers from an overflow.
    pub fn can_finish(&self) -> bool {
        self.camera.ready || self.camera.is_stopped()
    }

    /// Hand this frame over: give its buffer away and take `spare` in
    /// exchange. The returned buffer holds the frame, [`WIDTH`] x
    /// [`HEIGHT`] pixels of big-endian RGB565, row after row.
    ///
    /// Use it for a frame that a long computation works on: nothing is
    /// copied. Until the next frame comes, the camera has no current
    /// frame ([`Camera::current`] is `None`): its buffer holds what `spare`
    /// held, for example the frame taken before. With
    /// [`Camera::begin_frame`], draw a frame after `take` only when
    /// [`Frame::finish`] has brought a new one.
    ///
    /// # Panics
    ///
    /// When `spare` is not exactly one frame long.
    pub fn take(self, spare: &'static mut [u8]) -> &'static mut [u8] {
        assert_eq!(spare.len(), FRAME_BYTES, "a spare frame buffer");
        self.camera.display_whole = false;
        core::mem::replace(&mut self.camera.display_buffer, spare)
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
/// [`Camera::begin_frame`] or [`Camera::service`].
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
        on_demand: false,
        wanted: false,
        copying: true,
        aligning: false,
        display_whole: false,
        overflowed: false,
        last_pump: None,
        stats: CaptureStats::NONE,
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
        self.display_whole = false;
        self.overflowed = false;
        self.ready = false;
        self.filled = 0;
        self.wanted = false;
        self.short_frame = None;
    }

    /// Empty the ring, and start the capture when it is stopped: at the
    /// start, after [`Camera::pause`], after the ring overflowed. Never
    /// waits; see the [module documentation](super).
    ///
    /// Call it often: the ring holds a few milliseconds of the sensor's
    /// data. After a start, the first whole frame comes one to two frame
    /// periods later.
    pub fn service(&mut self) {
        self.pump_capture();
        if core::mem::take(&mut self.overflowed) {
            self.report_bad_frame(CaptureError::StreamEnded);
        }
        if self.is_stopped() {
            self.start_stream();
        }
    }

    /// Make the newest whole frame the current one. Returns whether there
    /// was a newer one. Never waits.
    pub fn advance(&mut self) -> bool {
        if !self.ready {
            return false;
        }
        self.show_ready_frame();
        true
    }

    /// The current frame: the newest whole frame that
    /// [`Camera::advance`] has seen. `None` before the first frame, and
    /// after the frame was handed over with [`Frame::take`].
    pub fn current(&mut self) -> Option<Frame<'_>> {
        (self.display_ready && self.display_whole).then_some(Frame { camera: self })
    }

    /// Frames dropped so far: by an overflow of the ring, by a wrong
    /// length, and, in [`Camera::begin_frame`] and [`Frame::finish`], by a
    /// sensor that sent nothing in time. [`Camera::service`] does not
    /// notice a silent sensor: no frame comes.
    pub fn bad_frames(&self) -> u32 {
        self.bad_frames
    }

    /// What the capture did since the last call of this function.
    pub fn take_stats(&mut self) -> CaptureStats {
        core::mem::replace(&mut self.stats, CaptureStats::NONE)
    }

    /// Copy frames only on request (`true`), or every frame the sensor
    /// sends (`false`, as after start-up).
    ///
    /// On request means: a frame is copied when [`Camera::request_frame`]
    /// was called before the sensor began to send it. So a frame is ready
    /// one to two frame periods after the request. A [`Frame::finish`] that
    /// has to wait asks for a frame by itself.
    pub fn capture_on_demand(&mut self, on_demand: bool) {
        self.on_demand = on_demand;
    }

    /// Ask for a frame newer than the current one. When one is ready, or
    /// on its way into the capture buffer, that one is it; otherwise the
    /// next frame that the sensor begins is copied. When the frame on its
    /// way is lost (the ring overflowed, or the frame had the wrong
    /// length), the next one is copied instead. Without
    /// [`Camera::capture_on_demand`], every frame is copied and this call
    /// changes nothing.
    ///
    /// A caller that asks again while its frame is on its way does not get
    /// a second frame copied: a copy costs CPU0 about 10 ms.
    pub fn request_frame(&mut self) {
        let on_its_way = self.copying && !self.aligning && !self.is_stopped();
        if !self.ready && !on_its_way {
            self.wanted = true;
        }
    }

    /// Copy the data that the sensor has sent so far for the next frame.
    /// Never waits. Does nothing while the camera is paused, and before the
    /// first frame of [`Camera::begin_frame`] or [`Camera::advance`].
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
            self.aligning = false;
            self.last_pump = None;
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
        let now = Instant::now();
        if let Some(last) = self.last_pump.replace(now) {
            self.stats.longest_gap = self.stats.longest_gap.max(now - last);
        }

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
            if self.aligning {
                // The rest of the frame that was on its way when the
                // stream started: not a frame.
                transfer.consume(take);
                if eof && take == available {
                    self.aligning = false;
                    self.filled = 0;
                    self.copying = !self.on_demand || core::mem::take(&mut self.wanted);
                }
                continue;
            }
            let copy_len = take.min(FRAME_BYTES.saturating_sub(self.filled));
            // After a missed VSYNC, the frame is longer than the buffer. Such
            // bytes are only counted, so the frame is reported as too long and
            // dropped. The bytes of a frame that nobody asked for are only
            // counted too.
            if self.copying && copy_len != 0 {
                let started = Instant::now();
                self.capture_buffer[self.filled..self.filled + copy_len]
                    .copy_from_slice(&chunk[..copy_len]);
                self.stats.copy_time += started.elapsed();
            }
            self.filled = self.filled.saturating_add(take);
            // `consume(0)` is still necessary: it gives an empty descriptor
            // back to the DMA. Such a descriptor carries only the VSYNC flag.
            transfer.consume(take);
            if eof && take == available {
                // The frame ended. A whole frame moves to the ready buffer.
                // A frame with the wrong length is dropped. In both cases,
                // capture continues with the next frame.
                if self.copying {
                    if self.filled == FRAME_BYTES {
                        core::mem::swap(&mut self.capture_buffer, &mut self.ready_buffer);
                        self.ready = true;
                        self.stats.frames_copied += 1;
                    } else {
                        self.short_frame = Some(self.filled);
                        // The frame that was asked for is lost: the next
                        // one takes its place.
                        self.wanted = true;
                    }
                }
                self.filled = 0;
                // The next frame begins here: is it wanted?
                self.copying = !self.on_demand || core::mem::take(&mut self.wanted);
            }
        }

        if transfer.is_done() {
            // The ring overflowed and the DMA transfer stopped.
            // `finish_capture` reports it, and the next `begin_frame` starts
            // a new capture; or `service` does both. A frame that was asked
            // for and on its way is lost: the next one takes its place.
            if self.copying && !self.aligning {
                self.wanted = true;
            }
            self.stop_stream();
            self.overflowed = true;
            self.last_pump = None;
        }
    }

    /// Start the DMA transfer, without waiting for the sensor: the bytes
    /// up to the next frame boundary are thrown away as they come. After an
    /// error the stream stays stopped, and the next [`Camera::service`]
    /// tries again.
    fn start_stream(&mut self) {
        self.stop_stream();
        let Some(Stream::Stopped { driver, buffer }) = self.stream.take() else {
            unreachable!("stream was stopped above");
        };
        self.stream = Some(match driver.receive(buffer) {
            Ok(transfer) => {
                self.aligning = true;
                self.filled = 0;
                self.last_pump = None;
                Stream::Running(transfer)
            }
            Err((error, driver, buffer)) => {
                warn!("Camera DMA start failed: {:?}", error);
                self.report_bad_frame(CaptureError::DmaStart);
                Stream::Stopped { driver, buffer }
            }
        });
    }

    /// Make the frame in the ready buffer the current one. When a frame
    /// with the wrong length came before it, log that frame now.
    fn show_ready_frame(&mut self) {
        self.ready = false;
        core::mem::swap(&mut self.display_buffer, &mut self.ready_buffer);
        self.display_ready = true;
        self.display_whole = true;
        if let Some(bytes) = self.short_frame.take() {
            // Logging blocks for milliseconds, and the sensor continues to
            // send data during that time. So the log message can cost the
            // frame in progress. Only some bad frames are logged, so this
            // happens rarely.
            self.report_bad_frame(CaptureError::BadLength(bytes));
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
                self.show_ready_frame();
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
                self.display_whole = false;
                self.overflowed = false;
                self.filled = 0;
                self.short_frame = None;
                return Err(error);
            }
            // The caller waits for a frame, so it wants one.
            self.wanted = true;
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
        // The stream now stands at the beginning of a frame, and the
        // caller waits for it.
        self.copying = true;
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
