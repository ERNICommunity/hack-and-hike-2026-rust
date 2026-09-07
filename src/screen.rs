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

use crate::{
    board, live_views,
    models::{ImuDisplay, ViewId},
    theme, waveform,
};

const SCREEN_WIDTH: usize = 320;
const SCREEN_HEIGHT: usize = 240;
const NAV_WIDTH: usize = live_views::CONTENT_X;
const NAV_BUTTON_HEIGHT: usize = SCREEN_HEIGHT / ViewId::ALL.len();
const DISPLAY_SPI_MHZ: u32 = 40;
const PIXEL_DMA_BYTES: usize = SCREEN_WIDTH * 2;
const CONTROL_DMA_BYTES: usize = 256;

const DCS_COLUMN_ADDRESS_SET: u8 = 0x2A;
const DCS_PAGE_ADDRESS_SET: u8 = 0x2B;
const DCS_MEMORY_WRITE: u8 = 0x2C;

type DisplaySpiDma = SpiDma<'static, Blocking>;
type DisplaySpiDmaBus = SpiDmaBus<'static, Blocking>;
type PixelTransfer = SpiDmaTransfer<'static, Blocking, DmaTxBuf>;
type Pixel = u16;

const NAV_ICONS: [[u16; 16]; 5] = [
    [0x0000,0x0000,0x0180,0x03C0,0x0660,0x0C30,0x1818,0x0180,0x0180,0x1818,0x0C30,0x0660,0x03C0,0x0180,0x0000,0x0000],
    [0x0180,0x0180,0x0180,0x0180,0x0180,0x7FFE,0x0180,0x0180,0x0180,0x0180,0x07E0,0x0DB0,0x198C,0x0180,0x0180,0x0000],
    [0x03C0,0x0660,0x0C30,0x0C30,0x0C30,0x0C30,0x0C30,0x0C30,0x0660,0x03C0,0x0180,0x1FF8,0x0180,0x0180,0x07E0,0x0000],
    [0x0000,0x0300,0x0700,0x0F18,0x7F0C,0x7F06,0x7F06,0x7F06,0x7F06,0x7F06,0x7F0C,0x0F18,0x0700,0x0300,0x0000,0x0000],
    [0x0000,0x0000,0x3FFC,0x2004,0x2FF4,0x2004,0x2FF4,0x2004,0x2FF4,0x2004,0x2FF4,0x2004,0x3FFC,0x0000,0x0000,0x0000],
];

pub struct Resources {
    pub spi2: SPI2<'static>,
    pub dma: DMA_CH1<'static>,
    pub sck: GPIO36<'static>,
    pub mosi: GPIO37<'static>,
    pub dc: GPIO35<'static>,
    pub cs: GPIO3<'static>,
}

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
        let control_rx = self.control_rx.take().expect("missing LCD control RX DMA buffer");
        let control_tx = self.control_tx.take().expect("missing LCD control TX DMA buffer");
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
            Ok(transfer) => self.state = Some(PipelineState::InFlight { transfer, free }),
            Err((err, spi, buffer)) => {
                let _ = self.cs.set_high();
                self.state = Some(PipelineState::Idle { spi, first: buffer, second: free });
                panic!("LCD pixel DMA start failed: {:?}", err);
            }
        }
    }

    fn queue_line(&mut self, line: usize, range: Range<usize>, pixels: &[Pixel]) {
        if range.is_empty() || pixels.is_empty() {
            return;
        }

        let state = self.state.take().expect("LCD DMA pipeline state missing");
        match state {
            PipelineState::Idle { spi, mut first, second } => {
                let byte_len = Self::encode_pixels(&mut first, pixels);
                let spi = self.set_window(spi, line, range);
                self.start_pixel_transfer(spi, first, byte_len, second);
            }
            PipelineState::InFlight { transfer, mut free } => {
                let byte_len = Self::encode_pixels(&mut free, pixels);
                let (spi, completed) = transfer.wait();
                self.cs.set_high();
                let spi = self.set_window(spi, line, range);
                self.start_pixel_transfer(spi, free, byte_len, completed);
            }
        }
    }

    fn finish(&mut self) {
        let Some(state) = self.state.take() else { return; };
        match state {
            PipelineState::Idle { .. } => self.state = Some(state),
            PipelineState::InFlight { transfer, free } => {
                let (spi, completed) = transfer.wait();
                self.cs.set_high();
                self.state = Some(PipelineState::Idle { spi, first: free, second: completed });
            }
        }
    }
}

pub struct Screen {
    pipeline: DisplayPipeline,
    line_buffer: [Pixel; SCREEN_WIDTH],
    live_frame: live_views::Framebuffer,
}

pub fn init(
    i2c: &mut impl embedded_hal::i2c::I2c,
    resources: Resources,
    delay: &mut Delay,
) -> Screen {
    let Resources { spi2, dma, sck, mosi, dc, cs } = resources;

    board::power::enable_lcd_backlight(i2c);
    board::io_expander::reset_display_and_touch(i2c, delay);

    let spi = Spi::new(
        spi2,
        SpiConfig::default().with_frequency(Rate::from_mhz(DISPLAY_SPI_MHZ)),
    )
    .unwrap()
    .with_sck(sck)
    .with_mosi(mosi)
    .with_dma(dma);

    let (rx_buffer, rx_descriptors, tx_buffer, tx_descriptors) = esp_hal::dma_buffers!(CONTROL_DMA_BYTES);
    let control_rx = DmaRxBuf::new(rx_descriptors, rx_buffer).unwrap();
    let control_tx = DmaTxBuf::new(tx_descriptors, tx_buffer).unwrap();
    let dma_bus = spi.with_buffers(control_rx, control_tx);

    let dc = Output::new(dc, Level::Low, OutputConfig::default());
    let cs = Output::new(cs, Level::High, OutputConfig::default());
    let spi_device = OwnedSpiDevice::new(dma_bus, cs).expect("Failed to initialize LCD SPI device");
    let di = display_interface_spi::SPIInterface::new(spi_device, dc);

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

    let first = esp_hal::dma_tx_buffer!(PIXEL_DMA_BYTES).expect("Could not init scan line DMA buffer 1");
    let second = esp_hal::dma_tx_buffer!(PIXEL_DMA_BYTES).expect("Could not init scan line DMA buffer 2");

    Screen {
        pipeline: DisplayPipeline::new(spi, control_rx, control_tx, first, second, cs, dc),
        line_buffer: [0; SCREEN_WIDTH],
        live_frame: live_views::Framebuffer::new(),
    }
}

const WAVEFORM_BACKGROUND: Pixel = theme::WHITE_RGB565;
const WAVEFORM_GRID: Pixel = theme::LIGHT_GRAY_RGB565;
const WAVEFORM_TRACE: Pixel = theme::DARK_BLUE_RGB565;

fn render_waveform_channel(
    pipeline: &mut DisplayPipeline,
    line_buffer: &mut [Pixel; SCREEN_WIDTH],
    top: usize,
    samples: &[i8; waveform::POINTS],
) {
    let x_start = waveform::CANVAS_X;
    let x_end = x_start + waveform::CANVAS_WIDTH;

    for local_y in 0..waveform::CANVAS_HEIGHT {
        let pixels = &mut line_buffer[x_start..x_end];
        pixels.fill(WAVEFORM_BACKGROUND);
        if local_y as i32 == waveform::CENTER_Y {
            pixels.fill(WAVEFORM_GRID);
        }

        for point in 0..waveform::POINTS {
            let current_y = waveform::CENTER_Y - i32::from(samples[point]);
            let previous_y = if point == 0 { current_y } else { waveform::CENTER_Y - i32::from(samples[point - 1]) };
            let low = current_y.min(previous_y);
            let high = current_y.max(previous_y);
            if (local_y as i32) >= low && (local_y as i32) <= high {
                let x = point * 2;
                pixels[x] = WAVEFORM_TRACE;
                pixels[x + 1] = WAVEFORM_TRACE;
            }
        }

        pipeline.queue_line(top + local_y, x_start..x_end, pixels);
    }
}

impl Screen {
    pub fn render_view_shell(&mut self, view: ViewId) {
        self.render_navigation(view);
        match view {
            ViewId::Network => self.live_frame.render_placeholder("NETWORK", "Peer communication"),
            ViewId::Imu | ViewId::Log => self.live_frame.render_blank(),
            ViewId::Microphone => self.live_frame.render_microphone_shell(),
            ViewId::Sound => self.live_frame.render_placeholder("SOUND", "Speaker output"),
        }
        self.blit_live_frame();
    }

    pub fn render_waveform(&mut self, frame: &waveform::WaveformFrame) {
        let pipeline = &mut self.pipeline;
        let line_buffer = &mut self.line_buffer;
        render_waveform_channel(pipeline, line_buffer, waveform::LEFT_CANVAS_Y, &frame.left);
        render_waveform_channel(pipeline, line_buffer, waveform::RIGHT_CANVAS_Y, &frame.right);
        self.pipeline.finish();
    }

    pub fn render_imu(&mut self, imu: &ImuDisplay) {
        self.live_frame.render_imu(imu);
        self.blit_live_frame();
    }

    pub fn render_log(&mut self, text: &str) {
        self.live_frame.render_log(text);
        self.blit_live_frame();
    }

    fn render_navigation(&mut self, active: ViewId) {
        for y in 0..SCREEN_HEIGHT {
            let button_index = y / NAV_BUTTON_HEIGHT;
            let selected = ViewId::ALL[button_index] == active;
            let background = if selected { theme::LIGHT_BLUE_RGB565 } else { theme::DARK_BLUE_RGB565 };
            let foreground = if selected { theme::WHITE_RGB565 } else { theme::DARK_GRAY_RGB565 };

            let pixels = &mut self.line_buffer[..NAV_WIDTH];
            pixels.fill(background);
            pixels[NAV_WIDTH - 1] = theme::DARK_BLUE_RGB565;

            let local_y = y % NAV_BUTTON_HEIGHT;
            if (16..32).contains(&local_y) {
                let row_bits = NAV_ICONS[button_index][local_y - 16];
                for icon_x in 0..16 {
                    if row_bits & (1 << (15 - icon_x)) != 0 {
                        pixels[14 + icon_x] = foreground;
                    }
                }
            }

            self.pipeline.queue_line(y, 0..NAV_WIDTH, pixels);
        }
        self.pipeline.finish();
    }

    fn blit_live_frame(&mut self) {
        let frame = &self.live_frame;
        let pipeline = &mut self.pipeline;
        let line_buffer = &mut self.line_buffer;
        let x_start = live_views::CONTENT_X;
        let x_end = x_start + live_views::WIDTH;

        for y in 0..live_views::HEIGHT {
            let source = &frame.pixels()[y * live_views::WIDTH..(y + 1) * live_views::WIDTH];
            let destination = &mut line_buffer[x_start..x_end];
            destination.copy_from_slice(source);
            pipeline.queue_line(y, x_start..x_end, destination);
        }
        self.pipeline.finish();
    }
}
