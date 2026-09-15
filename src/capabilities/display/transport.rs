//! Sending pixels to the ILI9342C panel controller over SPI with DMA.
//!
//! The one-time setup of the controller is in [`super::controller`]. This
//! module does the work after the setup: it sets a drawing window and sends
//! pixel bytes into it. It allocates no memory while it draws.
//!
//! # How a rectangle is sent
//!
//! 1. [`Transport::render`] sets the controller's *window* with DCS commands
//!    (Display Command Set, the MIPI standard commands for displays). One
//!    command sets the columns, one sets the rows ("pages"), and a third
//!    command starts the memory write. The controller then fills that
//!    rectangle with the bytes that follow: left to right, top to bottom.
//! 2. Rows go out in batches of [`BATCH_LINES`]. The source fills the free
//!    DMA buffer. Then `send` waits until the previous batch has left the
//!    bus, and starts the new batch.
//! 3. Two DMA buffers take turns: the free buffer and the buffer that is
//!    being sent. So the CPU prepares the next batch while the previous batch
//!    is still being sent. While `send` waits, the source's
//!    `while_transferring` callback runs. The camera uses it to capture its
//!    next frame.

use embedded_graphics::primitives::Rectangle;
use embedded_hal::spi::SpiBus as _;
use esp_hal::{
    Blocking,
    delay::Delay,
    dma::DmaTxBuf,
    gpio::{Level, Output, OutputConfig},
    spi::master::{Config as SpiConfig, Spi, SpiDma, SpiDmaTransfer},
    time::Rate,
};

use super::{BYTES_PER_PIXEL, Resources, ScanlineSource, WIDTH, controller};

/// The SPI clock in MHz. It limits the drawing speed: a full frame is
/// 153,600 bytes and takes about 31 ms at 40 MHz.
///
/// The ESP32-S3 can run SPI at 80 MHz, but not on the CoreS3 Lite. With
/// 80 MHz for the setup commands, the panel stays dark. With 80 MHz only for
/// the pixel data, the picture shows noise. Both were tested on 2026-09-14.
/// Keep 40 MHz, and redraw only small areas instead (see `ui::Canvas`).
const SPI_MHZ: u32 = 40;

/// Rows (scanlines) per DMA batch. Seven full-width rows are 4,480 bytes.
/// That is a little more than one DMA descriptor (`CHUNK_SIZE`, 4,092
/// bytes). Fewer, larger batches have less overhead per transfer. This helps
/// sources that send whole frames, such as the camera.
const BATCH_LINES: usize = 7;
/// Size of each of the two pixel DMA buffers.
const BATCH_BYTES: usize = WIDTH * BYTES_PER_PIXEL * BATCH_LINES;
/// Size of the DMA buffers that `mipidsi` uses for the setup commands.
const CONTROL_DMA_BYTES: usize = 256;

// MIPI DCS commands that are used after the setup.
/// Set the first and last column of the drawing window.
const DCS_COLUMN_ADDRESS_SET: u8 = 0x2A;
/// Set the first and last row ("page") of the drawing window.
const DCS_PAGE_ADDRESS_SET: u8 = 0x2B;
/// Start to write pixels into the window. All data bytes after it are pixel
/// data, until the next command.
const DCS_MEMORY_WRITE: u8 = 0x2C;

/// The blocking SPI driver with DMA. The controller setup uses the same type.
type DisplaySpiDma = SpiDma<'static, Blocking>;
/// A DMA write in progress. It owns the SPI driver and the buffer that is
/// being sent, and gives both back when it is done.
type PixelTransfer = SpiDmaTransfer<'static, Blocking, DmaTxBuf>;

/// The SPI driver and the two pixel buffers, in one of two states.
///
/// The owner of the parts changes with the state. While a batch is being
/// sent, the transfer owns the SPI driver and one buffer. Only the other
/// buffer is free for the CPU to fill.
enum Pipeline {
    /// Nothing is being sent. Both buffers are available.
    Idle {
        /// The SPI driver, ready for the next command or batch.
        spi: DisplaySpiDma,
        /// The buffer that [`Transport::prepare`] gives out for the next
        /// batch.
        free: DmaTxBuf,
        /// The other buffer. It becomes `free` when the next batch is sent.
        spare: DmaTxBuf,
    },
    /// A batch is being sent. The CPU can fill `free` at the same time.
    InFlight {
        /// The running DMA write. It holds the SPI driver and the buffer that
        /// is being sent.
        transfer: PixelTransfer,
        /// The buffer that is not being sent. The CPU fills it with the next
        /// batch.
        free: DmaTxBuf,
    },
}

impl Pipeline {
    /// Wait until no batch is being sent. Call `while_transferring` again and
    /// again during the wait.
    ///
    /// Return the SPI driver, the buffer that was free, and the buffer that
    /// was just sent. When nothing was being sent, the third value is the
    /// spare buffer.
    fn drain(self, mut while_transferring: impl FnMut()) -> (DisplaySpiDma, DmaTxBuf, DmaTxBuf) {
        match self {
            Self::Idle { spi, free, spare } => (spi, free, spare),
            Self::InFlight { transfer, free } => {
                // A busy wait on purpose. A full batch takes only about 1 ms
                // at 40 MHz, and the callback does useful work during it.
                while !transfer.is_done() {
                    while_transferring();
                    core::hint::spin_loop();
                }
                let (spi, done) = transfer.wait();
                (spi, free, done)
            }
        }
    }
}

/// The LCD's SPI connection after the controller setup.
pub(super) struct Transport {
    /// The SPI driver and the pixel buffers. `None` only while a method
    /// moves the pipeline from one state to the other: the method takes the
    /// parts out of the enum and puts them back.
    pipeline: Option<Pipeline>,
    /// Chip select (CS), active low. It stays low during one command, or
    /// during all batches of one window.
    cs: Output<'static>,
    /// Data/command select (DC): low for a command byte, high for its data.
    dc: Output<'static>,
}

/// Configure the SPI peripheral, run the controller setup and allocate the
/// two pixel buffers.
///
/// # Panics
///
/// When the SPI configuration is invalid, a DMA buffer cannot be allocated,
/// or the controller setup fails.
pub(super) fn init(resources: Resources, delay: Delay) -> Transport {
    let Resources {
        spi2,
        dma,
        sck,
        mosi,
        dc,
        cs,
    } = resources;

    let config = SpiConfig::default().with_frequency(Rate::from_mhz(SPI_MHZ));
    let spi = Spi::new(spi2, config)
        .expect("LCD SPI configuration is valid")
        .with_sck(sck)
        .with_mosi(mosi)
        .with_dma(dma);

    let control_rx = esp_hal::dma_rx_buffer!(CONTROL_DMA_BYTES).expect("LCD control DMA buffer");
    let control_tx = esp_hal::dma_tx_buffer!(CONTROL_DMA_BYTES).expect("LCD control DMA buffer");
    let dma_bus = spi.with_buffers(control_rx, control_tx);

    let dc = Output::new(dc, Level::Low, OutputConfig::default());
    let cs = Output::new(cs, Level::High, OutputConfig::default());
    let controller::Initialized { spi, cs, dc } = controller::initialize(dma_bus, cs, dc, delay);

    let free = esp_hal::dma_tx_buffer!(BATCH_BYTES).expect("LCD pixel DMA buffer");
    let spare = esp_hal::dma_tx_buffer!(BATCH_BYTES).expect("LCD pixel DMA buffer");

    Transport {
        pipeline: Some(Pipeline::Idle { spi, free, spare }),
        cs,
        dc,
    }
}

impl Transport {
    /// Draw `area` (in panel coordinates) from `source`. Return when the last
    /// batch has reached the panel. For an empty area, send nothing.
    ///
    /// # Panics
    ///
    /// When a corner of `area` has a negative coordinate, or an SPI command
    /// or a DMA transfer fails.
    pub(super) fn render(&mut self, area: Rectangle, source: &mut impl ScanlineSource) {
        let Some(bottom_right) = area.bottom_right() else {
            return;
        };
        let width = area.size.width as usize;
        let height = area.size.height as usize;
        let row_bytes = width * BYTES_PER_PIXEL;

        self.begin_window(
            [area.top_left.x, area.top_left.y].map(panel_coordinate),
            [bottom_right.x, bottom_right.y].map(panel_coordinate),
        );
        for first_row in (0..height).step_by(BATCH_LINES) {
            let rows = (height - first_row).min(BATCH_LINES);
            let buffer = self.prepare();
            for (offset, row) in buffer[..rows * row_bytes]
                .chunks_exact_mut(row_bytes)
                .enumerate()
            {
                source.fill_row(first_row + offset, row);
            }
            self.send(rows * row_bytes, || source.while_transferring());
        }
        self.finish(|| source.while_transferring());
    }

    /// Take the pipeline out of `self.pipeline`. Every caller puts it back
    /// before it returns.
    fn take_pipeline(&mut self) -> Pipeline {
        self.pipeline
            .take()
            .expect("LCD pipeline is only empty inside Transport methods")
    }

    /// Send one DCS command byte with DC low, then its parameter bytes with
    /// DC high. Chip select is low during the command and high after it.
    ///
    /// # Panics
    ///
    /// When an SPI write fails.
    fn write_command(&mut self, bus: &mut DisplaySpiDma, command: u8, data: &[u8]) {
        self.cs.set_low();
        self.dc.set_low();

        bus.write(&[command]).expect("LCD command write failed");
        bus.flush().expect("LCD command write failed");

        if !data.is_empty() {
            self.dc.set_high();
            bus.write(data).expect("LCD command write failed");
            bus.flush().expect("LCD command write failed");
        }

        self.cs.set_high();
    }

    /// Set a rectangular window from the top-left pixel to the bottom-right
    /// pixel, both included, and start a memory write into it.
    ///
    /// The controller moves through the window by itself. So after this
    /// call, the pixel path only sends RGB565 bytes one after the other.
    /// When a batch is still being sent, this function first waits for it.
    ///
    /// # Panics
    ///
    /// When an SPI write fails.
    fn begin_window(&mut self, [x0, y0]: [u16; 2], [x1, y1]: [u16; 2]) {
        let (mut spi, free, spare) = self.take_pipeline().drain(|| {});
        self.cs.set_high();

        let [x0_high, x0_low] = x0.to_be_bytes();
        let [x1_high, x1_low] = x1.to_be_bytes();
        let [y0_high, y0_low] = y0.to_be_bytes();
        let [y1_high, y1_low] = y1.to_be_bytes();
        self.write_command(
            &mut spi,
            DCS_COLUMN_ADDRESS_SET,
            &[x0_high, x0_low, x1_high, x1_low],
        );
        self.write_command(
            &mut spi,
            DCS_PAGE_ADDRESS_SET,
            &[y0_high, y0_low, y1_high, y1_low],
        );
        self.write_command(&mut spi, DCS_MEMORY_WRITE, &[]);

        self.pipeline = Some(Pipeline::Idle { spi, free, spare });
    }

    /// The free DMA buffer. Fill at most [`BATCH_BYTES`] bytes of it with
    /// RGB565 pixel data, most significant byte first, and then call `send`.
    fn prepare(&mut self) -> &mut [u8] {
        match self.pipeline.as_mut() {
            Some(Pipeline::Idle { free, .. } | Pipeline::InFlight { free, .. }) => {
                free.as_mut_slice()
            }
            None => unreachable!("LCD pipeline is only empty inside Transport methods"),
        }
    }

    /// Start to send the first `byte_len` bytes of the prepared buffer. Do
    /// nothing when `byte_len` is 0.
    ///
    /// First wait until the previous batch has left the bus. During this
    /// wait, call `while_transferring` again and again, so the caller can do
    /// useful work. Return as soon as the new transfer has started; do not
    /// wait for it.
    ///
    /// # Panics
    ///
    /// When the DMA transfer does not start.
    fn send(&mut self, byte_len: usize, while_transferring: impl FnMut()) {
        debug_assert!(byte_len <= BATCH_BYTES);
        if byte_len == 0 {
            return;
        }

        let (spi, mut buffer, free) = self.take_pipeline().drain(while_transferring);
        buffer.set_length(byte_len);

        self.dc.set_high();
        self.cs.set_low();
        let transfer = match spi.write_buffer(byte_len, buffer) {
            Ok(transfer) => transfer,
            Err((error, _, _)) => panic!("LCD pixel DMA start failed: {:?}", error),
        };
        self.pipeline = Some(Pipeline::InFlight { transfer, free });
    }

    /// Wait until the last batch of the current window has reached the panel,
    /// calling `while_transferring` during the wait. Then set chip select
    /// high, which ends the memory write.
    fn finish(&mut self, while_transferring: impl FnMut()) {
        let (spi, free, spare) = self.take_pipeline().drain(while_transferring);
        self.cs.set_high();
        self.pipeline = Some(Pipeline::Idle { spi, free, spare });
    }
}

/// Convert a panel coordinate into the controller's 16-bit address.
///
/// # Panics
///
/// When `value` is negative or larger than `u16::MAX`.
fn panel_coordinate(value: i32) -> u16 {
    u16::try_from(value).expect("surface coordinates fit the panel")
}
