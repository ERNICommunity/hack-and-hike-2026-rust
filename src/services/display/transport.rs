//! Pipelined SPI-DMA transport for the initialized ILI9342C.
//!
//! One-time controller setup lives in `controller`; this module owns only the
//! allocation-free steady-state DCS windowing and pixel DMA pipeline.

use core::ops::Range;

use embedded_hal::spi::SpiBus as _;
use esp_hal::{
    Blocking,
    delay::Delay,
    dma::{DmaRxBuf, DmaTxBuf},
    gpio::{Level, Output, OutputConfig},
    spi::master::{Config as SpiConfig, Spi, SpiDma, SpiDmaBus, SpiDmaTransfer},
    time::Rate,
};

use super::{Pixel, Resources, WIDTH, controller};

const DISPLAY_SPI_MHZ: u32 = 40;
pub(super) const RAW_BATCH_LINES: usize = 7;
const PIXEL_DMA_BYTES: usize = WIDTH * 2 * RAW_BATCH_LINES;
const CONTROL_DMA_BYTES: usize = 256;

const DCS_COLUMN_ADDRESS_SET: u8 = 0x2A;
const DCS_PAGE_ADDRESS_SET: u8 = 0x2B;
const DCS_MEMORY_WRITE: u8 = 0x2C;

type DisplaySpiDma = SpiDma<'static, Blocking>;
type DisplaySpiDmaBus = SpiDmaBus<'static, Blocking>;
type PixelTransfer = SpiDmaTransfer<'static, Blocking, DmaTxBuf>;

enum PipelineState {
    Idle {
        spi: DisplaySpiDma,
        first: DmaTxBuf,
        second: DmaTxBuf,
    },
    InFlight {
        transfer: PixelTransfer,
        free: DmaTxBuf,
    },
}

/// Ping-pong DMA state. While one chunk is leaving SPI, the caller can prepare
/// the next chunk in the second static DMA buffer.
pub(super) struct Transport {
    state: Option<PipelineState>,
    control_rx: Option<DmaRxBuf>,
    control_tx: Option<DmaTxBuf>,
    cs: Output<'static>,
    dc: Output<'static>,
}

pub(super) fn init(resources: Resources, delay: &mut Delay) -> Transport {
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
    .unwrap()
    .with_sck(sck)
    .with_mosi(mosi)
    .with_dma(dma);

    let (rx_buffer, rx_descriptors, tx_buffer, tx_descriptors) =
        esp_hal::dma_buffers!(CONTROL_DMA_BYTES);
    let control_rx = DmaRxBuf::new(rx_descriptors, rx_buffer).unwrap();
    let control_tx = DmaTxBuf::new(tx_descriptors, tx_buffer).unwrap();
    let dma_bus = spi.with_buffers(control_rx, control_tx);

    let dc = Output::new(dc, Level::Low, OutputConfig::default());
    let cs = Output::new(cs, Level::High, OutputConfig::default());
    let controller::Initialized {
        spi,
        control_rx,
        control_tx,
        cs,
        dc,
    } = controller::initialize(dma_bus, cs, dc, delay);

    // Seven rows cut Camera pixel submissions from 60 to 35 per 240-row frame.
    // The centered 276-pixel Camera region is 3,864 bytes per full batch, below
    // a single 4 KiB GDMA payload; normal UI rendering still queues one row.
    let first =
        esp_hal::dma_tx_buffer!(PIXEL_DMA_BYTES).expect("Could not init pixel DMA buffer 1");
    let second =
        esp_hal::dma_tx_buffer!(PIXEL_DMA_BYTES).expect("Could not init pixel DMA buffer 2");

    Transport {
        state: Some(PipelineState::Idle {
            spi,
            first,
            second,
        }),
        control_rx: Some(control_rx),
        control_tx: Some(control_tx),
        cs,
        dc,
    }
}

impl Transport {
    fn write_command(&mut self, bus: &mut DisplaySpiDmaBus, command: u8, data: &[u8]) {
        self.cs.set_low();
        self.dc.set_low();

        bus.write(&[command]).expect("LCD command DMA failed");
        bus.flush().expect("LCD command flush failed");

        if !data.is_empty() {
            self.dc.set_high();
            bus.write(data).expect("LCD command-data DMA failed");
            bus.flush().expect("LCD command-data flush failed");
        }

        self.cs.set_high();
    }

    fn set_window(
        &mut self,
        spi: DisplaySpiDma,
        columns: Range<usize>,
        pages: Range<usize>,
    ) -> DisplaySpiDma {
        debug_assert!(!columns.is_empty());
        debug_assert!(!pages.is_empty());

        let control_rx = self
            .control_rx
            .take()
            .expect("missing LCD control RX DMA buffer");
        let control_tx = self
            .control_tx
            .take()
            .expect("missing LCD control TX DMA buffer");
        let mut bus = DisplaySpiDmaBus::new(spi, control_rx, control_tx);

        let x0 = columns.start as u16;
        let x1 = (columns.end - 1) as u16;
        let y0 = pages.start as u16;
        let y1 = (pages.end - 1) as u16;

        let columns = [(x0 >> 8) as u8, x0 as u8, (x1 >> 8) as u8, x1 as u8];
        let pages = [(y0 >> 8) as u8, y0 as u8, (y1 >> 8) as u8, y1 as u8];

        self.write_command(&mut bus, DCS_COLUMN_ADDRESS_SET, &columns);
        self.write_command(&mut bus, DCS_PAGE_ADDRESS_SET, &pages);
        self.write_command(&mut bus, DCS_MEMORY_WRITE, &[]);

        let (spi, control_rx, control_tx) = bus.split();
        self.control_rx = Some(control_rx);
        self.control_tx = Some(control_tx);
        spi
    }

    /// Program one rectangular GRAM window before any of its pixel chunks are
    /// queued. The controller auto-increments through that window, so the pixel
    /// path only needs to stream consecutive RGB565 bytes afterwards.
    pub(super) fn begin_region(&mut self, columns: Range<usize>, pages: Range<usize>) {
        if columns.is_empty() || pages.is_empty() {
            return;
        }

        let state = self.state.take().expect("LCD DMA pipeline state missing");
        let PipelineState::Idle {
            spi,
            first,
            second,
        } = state
        else {
            self.state = Some(state);
            panic!("LCD region started while pixel DMA was still in flight");
        };

        let spi = self.set_window(spi, columns, pages);
        self.state = Some(PipelineState::Idle {
            spi,
            first,
            second,
        });
    }

    fn encode_pixels(buffer: &mut DmaTxBuf, pixels: &[Pixel]) -> usize {
        let byte_len = pixels.len() * 2;
        let bytes = &mut buffer.as_mut_slice()[..byte_len];

        for (dst, value) in bytes.chunks_exact_mut(2).zip(pixels.iter().copied()) {
            dst[0] = (value >> 8) as u8;
            dst[1] = value as u8;
        }

        buffer.set_length(byte_len);
        byte_len
    }

    fn copy_bytes(buffer: &mut DmaTxBuf, bytes: &[u8]) -> usize {
        let byte_len = bytes.len();
        debug_assert!(byte_len <= PIXEL_DMA_BYTES);
        buffer.as_mut_slice()[..byte_len].copy_from_slice(bytes);
        buffer.set_length(byte_len);
        byte_len
    }

    fn start_pixel_transfer(
        &mut self,
        spi: DisplaySpiDma,
        buffer: DmaTxBuf,
        byte_len: usize,
        free: DmaTxBuf,
    ) {
        self.dc.set_high();
        self.cs.set_low();

        match spi.write(byte_len, buffer) {
            Ok(transfer) => {
                self.state = Some(PipelineState::InFlight { transfer, free });
            }
            Err((err, spi, buffer)) => {
                let _ = self.cs.set_high();
                self.state = Some(PipelineState::Idle {
                    spi,
                    first: buffer,
                    second: free,
                });
                panic!("LCD pixel DMA start failed: {:?}", err);
            }
        }
    }

    /// Queue one decoded RGB565 scanline inside the active window.
    pub(super) fn queue_line(&mut self, pixels: &[Pixel]) {
        if pixels.is_empty() {
            return;
        }

        let state = self.state.take().expect("LCD DMA pipeline state missing");
        match state {
            PipelineState::Idle {
                spi,
                mut first,
                second,
            } => {
                let byte_len = Self::encode_pixels(&mut first, pixels);
                self.start_pixel_transfer(spi, first, byte_len, second);
            }
            PipelineState::InFlight { transfer, mut free } => {
                let byte_len = Self::encode_pixels(&mut free, pixels);
                let (spi, completed) = transfer.wait();
                self.start_pixel_transfer(spi, free, byte_len, completed);
            }
        }
    }

    /// Queue one raw RGB565 byte batch. While the prior SPI-DMA transfer is
    /// shifting pixels to the panel, `pump` can advance an independent producer
    /// such as the next camera frame.
    pub(super) fn queue_bytes_pumped(&mut self, bytes: &[u8], mut pump: impl FnMut()) {
        if bytes.is_empty() {
            return;
        }
        debug_assert!(bytes.len() <= PIXEL_DMA_BYTES);

        let state = self.state.take().expect("LCD DMA pipeline state missing");
        match state {
            PipelineState::Idle {
                spi,
                mut first,
                second,
            } => {
                let byte_len = Self::copy_bytes(&mut first, bytes);
                self.start_pixel_transfer(spi, first, byte_len, second);
            }
            PipelineState::InFlight { transfer, mut free } => {
                let byte_len = Self::copy_bytes(&mut free, bytes);
                while !transfer.is_done() {
                    pump();
                    core::hint::spin_loop();
                }
                pump();
                let (spi, completed) = transfer.wait();
                self.start_pixel_transfer(spi, free, byte_len, completed);
            }
        }
    }

    pub(super) fn finish(&mut self) {
        let Some(state) = self.state.take() else {
            return;
        };

        match state {
            PipelineState::Idle { .. } => self.state = Some(state),
            PipelineState::InFlight { transfer, free } => {
                let (spi, completed) = transfer.wait();
                self.cs.set_high();
                self.state = Some(PipelineState::Idle {
                    spi,
                    first: free,
                    second: completed,
                });
            }
        }
    }

    /// Finish the final LCD transfer while continuing to pump an independent
    /// producer until the last SPI byte of the current region has left the panel.
    pub(super) fn finish_pumped(&mut self, mut pump: impl FnMut()) {
        let Some(state) = self.state.take() else {
            return;
        };

        match state {
            PipelineState::Idle { .. } => self.state = Some(state),
            PipelineState::InFlight { transfer, free } => {
                while !transfer.is_done() {
                    pump();
                    core::hint::spin_loop();
                }
                pump();
                let (spi, completed) = transfer.wait();
                self.cs.set_high();
                self.state = Some(PipelineState::Idle {
                    spi,
                    first: free,
                    second: completed,
                });
            }
        }
    }
}
