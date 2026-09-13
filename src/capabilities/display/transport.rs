//! Pipelined SPI-DMA transport for the initialized ILI9342C.
//!
//! One-time controller setup lives in `controller`; this module owns only the
//! allocation-free steady-state DCS windowing and pixel DMA pipeline.
//!
//! Sending works in batches of rows: the source fills the free DMA buffer,
//! then `send` waits for the previous batch to leave the SPI bus and starts
//! the new one. Two buffers alternate, so the next batch is prepared while the
//! current one is in flight.

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

const DISPLAY_SPI_MHZ: u32 = 40;
/// Scanlines per DMA batch. Seven full-width rows are 4,480 bytes, which keeps
/// a batch close to one 4 KiB GDMA descriptor while cutting per-transfer
/// overhead for full-frame producers such as the camera.
const BATCH_LINES: usize = 7;
const BATCH_BYTES: usize = WIDTH * BYTES_PER_PIXEL * BATCH_LINES;
const CONTROL_DMA_BYTES: usize = 256;

const DCS_COLUMN_ADDRESS_SET: u8 = 0x2A;
const DCS_PAGE_ADDRESS_SET: u8 = 0x2B;
const DCS_MEMORY_WRITE: u8 = 0x2C;

type DisplaySpiDma = SpiDma<'static, Blocking>;
type PixelTransfer = SpiDmaTransfer<'static, Blocking, DmaTxBuf>;

enum Pipeline {
    Idle {
        spi: DisplaySpiDma,
        free: DmaTxBuf,
        spare: DmaTxBuf,
    },
    InFlight {
        transfer: PixelTransfer,
        free: DmaTxBuf,
    },
}

impl Pipeline {
    /// Wait until nothing is in flight, calling `while_transferring` meanwhile.
    fn drain(self, mut while_transferring: impl FnMut()) -> (DisplaySpiDma, DmaTxBuf, DmaTxBuf) {
        match self {
            Self::Idle { spi, free, spare } => (spi, free, spare),
            Self::InFlight { transfer, free } => {
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

pub(super) struct Transport {
    // `None` only while a method is moving the pipeline between states.
    pipeline: Option<Pipeline>,
    cs: Output<'static>,
    dc: Output<'static>,
}

pub(super) fn init(resources: Resources, delay: Delay) -> Transport {
    let Resources {
        spi2,
        dma,
        sck,
        mosi,
        dc,
        cs,
    } = resources;

    let spi = Spi::new(
        spi2,
        SpiConfig::default().with_frequency(Rate::from_mhz(DISPLAY_SPI_MHZ)),
    )
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
    fn take_pipeline(&mut self) -> Pipeline {
        self.pipeline
            .take()
            .expect("LCD pipeline is only empty inside Transport methods")
    }

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

    /// Draw `area` from `source`, waiting for the last batch to reach the
    /// panel before returning. Nothing is sent for an empty area.
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

    /// Program one rectangular GRAM window from the top-left to the
    /// bottom-right pixel, both inclusive. The controller auto-increments
    /// through that window, so the pixel path only streams consecutive RGB565
    /// bytes afterwards. Any batch still in flight is completed first.
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

    /// Wait for the last batch of the current region to reach the panel.
    fn finish(&mut self, while_transferring: impl FnMut()) {
        let (spi, free, spare) = self.take_pipeline().drain(while_transferring);
        self.cs.set_high();
        self.pipeline = Some(Pipeline::Idle { spi, free, spare });
    }
}

fn panel_coordinate(value: i32) -> u16 {
    u16::try_from(value).expect("surface coordinates fit the panel")
}
