//! ILI9342C initialization and pipelined SPI-DMA transport.
//!
//! This is intentionally private to `display`: presentation code cannot issue
//! DCS commands or take ownership of DMA buffers.

use core::ops::Range;

use embedded_hal::{
    delay::DelayNs as _,
    spi::{ErrorType as SpiErrorType, Operation, SpiBus, SpiDevice},
};
use embedded_hal_bus::spi::DeviceError;
use esp_hal::{
    Blocking,
    delay::Delay,
    dma::{DmaRxBuf, DmaTxBuf},
    gpio::{Level, Output, OutputConfig},
    spi::master::{Config as SpiConfig, Spi, SpiDma, SpiDmaBus, SpiDmaTransfer},
    time::Rate,
};
use mipidsi::options::{
    HorizontalRefreshOrder, Orientation, RefreshOrder, Rotation, VerticalRefreshOrder,
};

use crate::board;

use super::{Pixel, Resources, WIDTH};

const DISPLAY_SPI_MHZ: u32 = 40;
pub(super) const RAW_BATCH_LINES: usize = 4;
const PIXEL_DMA_BYTES: usize = WIDTH * 2 * RAW_BATCH_LINES;
const CONTROL_DMA_BYTES: usize = 256;

const DCS_COLUMN_ADDRESS_SET: u8 = 0x2A;
const DCS_PAGE_ADDRESS_SET: u8 = 0x2B;
const DCS_MEMORY_WRITE: u8 = 0x2C;
const DCS_FRAME_RATE_CONTROL_NORMAL: u8 = 0xB1;
const DCS_BLANKING_PORCH_CONTROL: u8 = 0xB5;
const DCS_SET_EXTC: u8 = 0xC8;

// ILI9342C level-2 timing commands require EXTC to be enabled. Keep normal
// operation at the controller defaults. During sustained camera streaming, use
// the maximum legal vertical front/back porches. With RTNA=0x1c the datasheet
// rates the default 2+2 porch configuration at roughly 61 Hz; scaling the total
// vertical period from 244 to 494 lines yields about 30 Hz. A 276x240 RGB565
// update takes about 26.5 ms at the board-proven 40 MHz SPI rate, so it can fit
// inside that longer display period instead of being guaranteed to cross a
// faster panel scan on every frame.
const EXTC_ENABLE: [u8; 3] = [0xFF, 0x93, 0x42];
const NORMAL_FRAME_RATE: [u8; 2] = [0x00, 0x1C];
const NORMAL_PORCH: [u8; 4] = [0x02, 0x02, 0x0A, 0x14];
const STREAMING_PORCH: [u8; 4] = [0x7F, 0x7F, 0x0A, 0x14];

type DisplaySpiDma = SpiDma<'static, Blocking>;
type DisplaySpiDmaBus = SpiDmaBus<'static, Blocking>;
type PixelTransfer = SpiDmaTransfer<'static, Blocking, DmaTxBuf>;

/// Small owned `SpiDevice` adapter used only during `mipidsi` initialization.
/// It can be deconstructed afterwards so steady-state rendering can use the
/// raw pipelined SPI-DMA transport.
struct OwnedSpiDevice<BUS, CS> {
    bus: BUS,
    cs: CS,
}

impl<BUS, CS> OwnedSpiDevice<BUS, CS>
where
    CS: embedded_hal::digital::OutputPin,
{
    fn new(bus: BUS, mut cs: CS) -> Result<Self, CS::Error> {
        cs.set_high()?;
        Ok(Self { bus, cs })
    }

    fn release(self) -> (BUS, CS) {
        (self.bus, self.cs)
    }
}

impl<BUS, CS> SpiErrorType for OwnedSpiDevice<BUS, CS>
where
    BUS: SpiBus<u8>,
    CS: embedded_hal::digital::OutputPin,
{
    type Error = DeviceError<BUS::Error, CS::Error>;
}

impl<BUS, CS> SpiDevice<u8> for OwnedSpiDevice<BUS, CS>
where
    BUS: SpiBus<u8>,
    CS: embedded_hal::digital::OutputPin,
{
    fn transaction(&mut self, operations: &mut [Operation<'_, u8>]) -> Result<(), Self::Error> {
        self.cs.set_low().map_err(DeviceError::Cs)?;

        let mut result = Ok(());
        let mut delay = Delay::new();

        for operation in operations {
            let operation_result = match operation {
                Operation::Read(words) => self.bus.read(words),
                Operation::Write(words) => self.bus.write(words),
                Operation::Transfer(read, write) => self.bus.transfer(read, write),
                Operation::TransferInPlace(words) => self.bus.transfer_in_place(words),
                Operation::DelayNs(ns) => {
                    delay.delay_ns(*ns);
                    Ok(())
                }
            };

            if let Err(err) = operation_result {
                result = Err(DeviceError::Spi(err));
                break;
            }
        }

        if result.is_ok() {
            if let Err(err) = self.bus.flush() {
                result = Err(DeviceError::Spi(err));
            }
        }

        let cs_result = self.cs.set_high().map_err(DeviceError::Cs);
        match result {
            Err(err) => Err(err),
            Ok(()) => cs_result,
        }
    }
}

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
    let spi_device =
        OwnedSpiDevice::new(dma_bus, cs).expect("Failed to initialize LCD SPI device");
    let di = display_interface_spi::SPIInterface::new(spi_device, dc);

    // The board is mounted 180 degrees, so logical top-to-bottom/left-to-right
    // GRAM writes travel physically bottom-to-top/right-to-left. Match the
    // ILI9342C's panel refresh direction to that same physical direction. The
    // default refresh direction is the opposite, which maximizes the chance that
    // an asynchronous full-screen camera write crosses the panel's live scan and
    // exposes a moving horizontal tear boundary.
    let (orientation, refresh_order) = if board::DISPLAY_ROTATED_180 {
        (
            Orientation::new().rotate(Rotation::Deg180),
            RefreshOrder {
                vertical: VerticalRefreshOrder::BottomToTop,
                horizontal: HorizontalRefreshOrder::RightToLeft,
            },
        )
    } else {
        (
            Orientation::new(),
            RefreshOrder {
                vertical: VerticalRefreshOrder::TopToBottom,
                horizontal: HorizontalRefreshOrder::LeftToRight,
            },
        )
    };

    // Keep mipidsi for the known-good controller initialization sequence, then
    // recover the bus and pins for the allocation-free steady-state DMA path.
    let display = mipidsi::Builder::new(mipidsi::models::ILI9342CRgb565, di)
        .color_order(mipidsi::options::ColorOrder::Bgr)
        .invert_colors(mipidsi::options::ColorInversion::Inverted)
        .orientation(orientation)
        .refresh_order(refresh_order)
        .init(delay)
        .unwrap();

    let (di, _model, _reset) = display.release();
    let (spi_device, dc) = di.release();
    let (dma_bus, cs) = spi_device.release();
    let (spi, control_rx, control_tx) = dma_bus.split();

    // Four full-width rows fit comfortably inside one DMA buffer while keeping
    // each camera crop batch below a single 4095-byte GDMA descriptor. Normal UI
    // rendering still queues one row at a time using the same larger buffers.
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

    /// Switch between the normal UI timing and a long-blanking streaming timing.
    /// This is only called at view boundaries, when no pixel DMA is in flight.
    pub(super) fn set_streaming_refresh_mode(&mut self, enabled: bool) {
        let state = self.state.take().expect("LCD DMA pipeline state missing");
        let PipelineState::Idle {
            spi,
            first,
            second,
        } = state
        else {
            self.state = Some(state);
            panic!("LCD refresh timing changed while pixel DMA was still in flight");
        };

        let control_rx = self
            .control_rx
            .take()
            .expect("missing LCD control RX DMA buffer");
        let control_tx = self
            .control_tx
            .take()
            .expect("missing LCD control TX DMA buffer");
        let mut bus = DisplaySpiDmaBus::new(spi, control_rx, control_tx);

        // Re-enable the ILI9342C extension page explicitly instead of depending
        // on initialization state retained by a particular controller revision.
        self.write_command(&mut bus, DCS_SET_EXTC, &EXTC_ENABLE);
        self.write_command(
            &mut bus,
            DCS_FRAME_RATE_CONTROL_NORMAL,
            &NORMAL_FRAME_RATE,
        );
        self.write_command(
            &mut bus,
            DCS_BLANKING_PORCH_CONTROL,
            if enabled {
                &STREAMING_PORCH
            } else {
                &NORMAL_PORCH
            },
        );

        let (spi, control_rx, control_tx) = bus.split();
        self.control_rx = Some(control_rx);
        self.control_tx = Some(control_tx);
        self.state = Some(PipelineState::Idle {
            spi,
            first,
            second,
        });
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

    /// Queue already encoded big-endian RGB565 bytes inside the active window.
    /// The free ping-pong buffer is filled while the previous DMA transfer is
    /// still in flight, preserving the same CPU/SPI overlap as `queue_line`.
    pub(super) fn queue_bytes(&mut self, bytes: &[u8]) {
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
}
