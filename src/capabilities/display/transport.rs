//! Pipelined SPI DMA transport for the ILI9342C panel controller.
//!
//! One-time controller setup lives in [`super::controller`]; this module owns
//! the steady state: programming a drawing window and streaming pixel bytes
//! into it, without allocating.
//!
//! # How a region is sent
//!
//! 1. [`Transport::render`] programs the controller's *window* (column and
//!    page address) with three DCS commands. The controller then fills that
//!    rectangle left to right, top to bottom, from whatever bytes follow.
//! 2. Rows are sent in batches of [`BATCH_LINES`]. The source fills the free
//!    DMA buffer, `send` waits for the previous batch to leave the bus and
//!    starts the new one.
//! 3. Two DMA buffers alternate (`free` and the one in flight), so the CPU
//!    prepares the next batch while the previous one is still on the wire.
//!    While it waits, the source's `while_transferring` hook runs; the camera
//!    uses it to capture its next frame.

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

/// The SPI clock, and with it the ceiling on drawing speed: a full frame is
/// 153,600 bytes, about 31 ms at 40 MHz.
///
/// The ESP32-S3 can clock SPI at 80 MHz, but on the CoreS3 Lite that leaves
/// the panel dark when used for the setup commands, and fills the picture
/// with noise when used for pixel data only (both tried on 2026-09-14). Keep
/// 40 MHz; make redraws small instead (see `ui::Canvas`).
const SPI_MHZ: u32 = 40;

/// Scanlines per DMA batch. Seven full-width rows are 4,480 bytes, which keeps
/// a batch close to one 4 KiB GDMA descriptor while cutting per-transfer
/// overhead for full-frame producers such as the camera.
const BATCH_LINES: usize = 7;
/// Size of each of the two pixel DMA buffers.
const BATCH_BYTES: usize = WIDTH * BYTES_PER_PIXEL * BATCH_LINES;
/// Size of the DMA buffers `mipidsi` uses for the setup commands.
const CONTROL_DMA_BYTES: usize = 256;

// MIPI DCS commands used after setup.
/// Set the first and last column of the drawing window.
const DCS_COLUMN_ADDRESS_SET: u8 = 0x2A;
/// Set the first and last row ("page") of the drawing window.
const DCS_PAGE_ADDRESS_SET: u8 = 0x2B;
/// Start writing pixels into the window; every following data byte is pixel
/// data until the next command.
const DCS_MEMORY_WRITE: u8 = 0x2C;

type DisplaySpiDma = SpiDma<'static, Blocking>;
type PixelTransfer = SpiDmaTransfer<'static, Blocking, DmaTxBuf>;

/// The SPI driver and the two pixel buffers, in one of two states.
///
/// Ownership moves between the states: while a batch is in flight, the SPI
/// driver and one buffer live inside the transfer, and only the other buffer
/// is free for the CPU to fill.
enum Pipeline {
    /// Nothing on the bus; both buffers are available.
    Idle {
        spi: DisplaySpiDma,
        free: DmaTxBuf,
        spare: DmaTxBuf,
    },
    /// A batch is being sent; `free` can be filled in the meantime.
    InFlight {
        transfer: PixelTransfer,
        free: DmaTxBuf,
    },
}

impl Pipeline {
    /// Wait until nothing is in flight, calling `while_transferring` meanwhile.
    ///
    /// Returns the SPI driver, the buffer that was free and the buffer that
    /// has just been sent (or the spare one when nothing was in flight).
    fn drain(self, mut while_transferring: impl FnMut()) -> (DisplaySpiDma, DmaTxBuf, DmaTxBuf) {
        match self {
            Self::Idle { spi, free, spare } => (spi, free, spare),
            Self::InFlight { transfer, free } => {
                // A busy wait on purpose: the transfer takes a few
                // milliseconds, and the hook turns the wait into useful work.
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

/// The LCD's SPI connection after controller setup.
pub(super) struct Transport {
    /// `None` only while a method is moving the pipeline between states, so
    /// the enum's contents can be moved out and back in.
    pipeline: Option<Pipeline>,
    /// Chip select, active low: frames one command or one pixel stream.
    cs: Output<'static>,
    /// Data/command select: low for a command byte, high for its data.
    dc: Output<'static>,
}

/// Configure the SPI peripheral, run the controller setup and allocate the
/// two pixel buffers.
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
    /// Draw `area` (panel coordinates) from `source`, and return once the
    /// last batch has reached the panel. Nothing is sent for an empty area.
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

    /// Take the pipeline out of its slot; every caller puts it back.
    fn take_pipeline(&mut self) -> Pipeline {
        self.pipeline
            .take()
            .expect("LCD pipeline is only empty inside Transport methods")
    }

    /// Send one DCS command byte followed by its parameter bytes.
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

    /// Program one rectangular window from the top-left to the bottom-right
    /// pixel, both inclusive, and start a memory write into it. The
    /// controller advances through the window by itself, so the pixel path
    /// only streams consecutive RGB565 bytes afterwards. Any batch still in
    /// flight is completed first.
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

    /// The free DMA buffer, to be filled with at most `BATCH_BYTES` of
    /// big-endian RGB565 pixel data before calling `send`.
    fn prepare(&mut self) -> &mut [u8] {
        match self.pipeline.as_mut() {
            Some(Pipeline::Idle { free, .. } | Pipeline::InFlight { free, .. }) => {
                free.as_mut_slice()
            }
            None => unreachable!("LCD pipeline is only empty inside Transport methods"),
        }
    }

    /// Send the first `byte_len` bytes of the prepared buffer. While the
    /// previous batch is still on the bus, `while_transferring` is called
    /// repeatedly so the caller can do useful work instead of waiting.
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

    /// Wait for the last batch of the current window to reach the panel.
    fn finish(&mut self, while_transferring: impl FnMut()) {
        let (spi, free, spare) = self.take_pipeline().drain(while_transferring);
        self.cs.set_high();
        self.pipeline = Some(Pipeline::Idle { spi, free, spare });
    }
}

/// A surface coordinate as the controller's 16-bit address.
fn panel_coordinate(value: i32) -> u16 {
    u16::try_from(value).expect("surface coordinates fit the panel")
}
