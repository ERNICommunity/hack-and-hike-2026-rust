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
    peripherals::{DMA_CH1, GPIO3, GPIO35, GPIO36, GPIO37, SPI2},
    spi::master::{Config as SpiConfig, Spi, SpiDma, SpiDmaBus, SpiDmaTransfer},
    time::Rate,
};
use slint::platform::software_renderer::{LineBufferProvider, MinimalSoftwareWindow, Rgb565Pixel};

const AXP2101_ADDR: u8 = 0x34;
const AW9523_ADDR: u8 = 0x58;

const SCREEN_WIDTH: usize = 320;
const PIXEL_DMA_BYTES: usize = SCREEN_WIDTH * 2;
const CONTROL_DMA_BYTES: usize = 256;

const DCS_COLUMN_ADDRESS_SET: u8 = 0x2A;
const DCS_PAGE_ADDRESS_SET: u8 = 0x2B;
const DCS_MEMORY_WRITE: u8 = 0x2C;

type DisplaySpiDma = SpiDma<'static, Blocking>;
type DisplaySpiDmaBus = SpiDmaBus<'static, Blocking>;
type PixelTransfer = SpiDmaTransfer<'static, Blocking, DmaTxBuf>;

/// Small owned SpiDevice adapter used only during mipidsi initialization.
///
/// Unlike embedded-hal-bus' ExclusiveDevice this adapter can be deconstructed,
/// which lets us recover the DMA bus and CS pin after mipidsi has configured
/// the ILI9342C. Steady-state rendering then uses the raw SPI-DMA engine.
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

fn update_register_bits(
    i2c: &mut impl embedded_hal::i2c::I2c,
    address: u8,
    register: u8,
    mask: u8,
    value: u8,
) {
    let mut current = [0u8; 1];
    if i2c.write_read(address, &[register], &mut current).is_ok() {
        let next = (current[0] & !mask) | (value & mask);
        let _ = i2c.write(address, &[register, next]);
    }
}

fn init_pmic_and_hardware_reset(i2c: &mut impl embedded_hal::i2c::I2c, delay: &mut Delay) {
    // LCD backlight rail (DLDO1) at 3.3 V. Microphone power is owned by audio.rs.
    let _ = i2c.write(AXP2101_ADDR, &[0x99, 0x1C]);

    let mut reg90 = [0u8; 1];
    if i2c.write_read(AXP2101_ADDR, &[0x90], &mut reg90).is_ok() {
        let _ = i2c.write(AXP2101_ADDR, &[0x90, reg90[0] | (1 << 7)]);
    }

    let _ = i2c.write(AW9523_ADDR, &[0x13, 0xFF]);

    // AW9523 direction registers: 0 = output, 1 = input.
    // P0_0 = FT6336 TOUCH_RST -> output.
    update_register_bits(i2c, AW9523_ADDR, 0x04, 1 << 0, 0);

    // P1_1 = LCD_RST -> output.
    // P1_2 = FT6336 TOUCH_INT -> input.
    update_register_bits(i2c, AW9523_ADDR, 0x05, (1 << 1) | (1 << 2), 1 << 2);

    // Reset LCD and touch controller together, preserving unrelated outputs.
    update_register_bits(i2c, AW9523_ADDR, 0x03, 1 << 1, 0);
    update_register_bits(i2c, AW9523_ADDR, 0x02, 1 << 0, 0);
    delay.delay_millis(20u32);

    update_register_bits(i2c, AW9523_ADDR, 0x03, 1 << 1, 1 << 1);
    update_register_bits(i2c, AW9523_ADDR, 0x02, 1 << 0, 1 << 0);

    delay.delay_millis(300u32);
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

/// CPU0-owned LCD transport.
///
/// One pixel buffer is transmitted by DMA while Slint renders/converts the
/// next scanline into the other buffer. The small command DMA buffers are
/// separate so DCS address-window commands can be sent between pixel transfers.
struct DisplayPipeline {
    state: Option<PipelineState>,
    control_rx: Option<DmaRxBuf>,
    control_tx: Option<DmaTxBuf>,
    cs: Output<'static>,
    dc: Output<'static>,
}

impl DisplayPipeline {
    fn new(
        spi: DisplaySpiDma,
        control_rx: DmaRxBuf,
        control_tx: DmaTxBuf,
        first: DmaTxBuf,
        second: DmaTxBuf,
        cs: Output<'static>,
        dc: Output<'static>,
    ) -> Self {
        Self {
            state: Some(PipelineState::Idle { spi, first, second }),
            control_rx: Some(control_rx),
            control_tx: Some(control_tx),
            cs,
            dc,
        }
    }

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
        line: usize,
        range: Range<usize>,
    ) -> DisplaySpiDma {
        let control_rx = self
            .control_rx
            .take()
            .expect("missing LCD control RX DMA buffer");
        let control_tx = self
            .control_tx
            .take()
            .expect("missing LCD control TX DMA buffer");
        let mut bus = DisplaySpiDmaBus::new(spi, control_rx, control_tx);

        let x0 = range.start as u16;
        let x1 = (range.end - 1) as u16;
        let y = line as u16;

        let columns = [(x0 >> 8) as u8, x0 as u8, (x1 >> 8) as u8, x1 as u8];
        let pages = [(y >> 8) as u8, y as u8, (y >> 8) as u8, y as u8];

        self.write_command(&mut bus, DCS_COLUMN_ADDRESS_SET, &columns);
        self.write_command(&mut bus, DCS_PAGE_ADDRESS_SET, &pages);
        self.write_command(&mut bus, DCS_MEMORY_WRITE, &[]);

        let (spi, control_rx, control_tx) = bus.split();
        self.control_rx = Some(control_rx);
        self.control_tx = Some(control_tx);
        spi
    }

    fn encode_pixels(buffer: &mut DmaTxBuf, pixels: &[Rgb565Pixel]) -> usize {
        let byte_len = pixels.len() * 2;
        let bytes = &mut buffer.as_mut_slice()[..byte_len];

        // Slint's Rgb565Pixel stores the same RGB565 bit layout expected by
        // the ILI9342C. The display wire format is MSB first, so byte-swap the
        // u16 directly instead of reconstructing embedded_graphics::Rgb565.
        for (dst, pixel) in bytes.chunks_exact_mut(2).zip(pixels.iter()) {
            let value = pixel.0;
            dst[0] = (value >> 8) as u8;
            dst[1] = value as u8;
        }

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

    fn queue_line(&mut self, line: usize, range: Range<usize>, pixels: &[Rgb565Pixel]) {
        if range.is_empty() || pixels.is_empty() {
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
                let spi = self.set_window(spi, line, range);
                self.start_pixel_transfer(spi, first, byte_len, second);
            }
            PipelineState::InFlight { transfer, mut free } => {
                // This render/conversion work happens while the previous line
                // is still physically leaving the SPI peripheral via DMA.
                let byte_len = Self::encode_pixels(&mut free, pixels);

                let (spi, completed) = transfer.wait();
                self.cs.set_high();

                let spi = self.set_window(spi, line, range);
                self.start_pixel_transfer(spi, free, byte_len, completed);
            }
        }
    }

    fn finish(&mut self) {
        let Some(state) = self.state.take() else {
            return;
        };

        match state {
            PipelineState::Idle { .. } => {
                self.state = Some(state);
            }
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

/// The one and only owner of SPI2, DMA_CH1, and the LCD.
///
/// Screen remains CPU0-only and mutex-free.
pub struct Screen {
    pipeline: DisplayPipeline,
}

pub fn init(
    i2c: &mut impl embedded_hal::i2c::I2c,
    spi2: SPI2<'static>,
    dma_ch1: DMA_CH1<'static>,
    gpio36: GPIO36<'static>,
    gpio37: GPIO37<'static>,
    gpio35: GPIO35<'static>,
    gpio3: GPIO3<'static>,
    delay: &mut Delay,
) -> Screen {
    init_pmic_and_hardware_reset(i2c, delay);

    let spi = Spi::new(
        spi2,
        SpiConfig::default().with_frequency(Rate::from_mhz(40)),
    )
    .unwrap()
    .with_sck(gpio36)
    .with_mosi(gpio37)
    .with_dma(dma_ch1);

    // Small internal DMA buffers back the blocking SpiDmaBus used for DCS
    // commands and for mipidsi's one-time controller initialization.
    let (rx_buffer, rx_descriptors, tx_buffer, tx_descriptors) =
        esp_hal::dma_buffers!(CONTROL_DMA_BYTES);
    let control_rx = DmaRxBuf::new(rx_descriptors, rx_buffer).unwrap();
    let control_tx = DmaTxBuf::new(tx_descriptors, tx_buffer).unwrap();
    let dma_bus = spi.with_buffers(control_rx, control_tx);

    let dc = Output::new(gpio35, Level::Low, OutputConfig::default());
    let cs = Output::new(gpio3, Level::High, OutputConfig::default());
    let spi_device = OwnedSpiDevice::new(dma_bus, cs).expect("Failed to initialize LCD SPI device");
    let di = display_interface_spi::SPIInterface::new(spi_device, dc);

    // Keep mipidsi for the complete, known-good ILI9342C initialization.
    // Afterwards release the resources and use our pipelined raw-DCS path.
    let display = mipidsi::Builder::new(mipidsi::models::ILI9342CRgb565, di)
        .color_order(mipidsi::options::ColorOrder::Bgr)
        .invert_colors(mipidsi::options::ColorInversion::Inverted)
        .orientation(mipidsi::options::Orientation::new())
        .init(delay)
        .unwrap();

    let (di, _model, _reset) = display.release();
    let (spi_device, dc) = di.release();
    let (dma_bus, cs) = spi_device.release();
    let (spi, control_rx, control_tx) = dma_bus.split();

    // Two line-sized static DMA buffers implement the render/transmit ping-pong.
    let first =
        esp_hal::dma_tx_buffer!(PIXEL_DMA_BYTES).expect("Could not init scan line DMA buffer 1");
    let second =
        esp_hal::dma_tx_buffer!(PIXEL_DMA_BYTES).expect("Could not init scan line DMA buffer 2");

    Screen {
        pipeline: DisplayPipeline::new(spi, control_rx, control_tx, first, second, cs, dc),
    }
}

struct DisplayWrapper<'a> {
    pipeline: &'a mut DisplayPipeline,
    line_buffer: &'a mut [Rgb565Pixel; SCREEN_WIDTH],
}

impl LineBufferProvider for DisplayWrapper<'_> {
    type TargetPixel = Rgb565Pixel;

    fn process_line(
        &mut self,
        line: usize,
        range: Range<usize>,
        render_fn: impl FnOnce(&mut [Self::TargetPixel]),
    ) {
        render_fn(&mut self.line_buffer[range.clone()]);
        self.pipeline
            .queue_line(line, range.clone(), &self.line_buffer[range]);
    }
}

impl Screen {
    /// CPU0-only Slint rendering with double-buffered SPI DMA.
    ///
    /// For every dirty scanline, Slint renders/converts the next line while the
    /// previous line is in flight on DMA_CH1. Only the final line of a redraw
    /// is explicitly waited for before returning.
    pub fn render_slint_window(&mut self, window: &MinimalSoftwareWindow) {
        let mut line_buffer = [Rgb565Pixel(0); SCREEN_WIDTH];

        window.draw_if_needed(|renderer| {
            renderer.render_by_line(DisplayWrapper {
                pipeline: &mut self.pipeline,
                line_buffer: &mut line_buffer,
            });
        });

        self.pipeline.finish();
    }
}
