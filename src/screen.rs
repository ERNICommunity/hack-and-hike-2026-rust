use core::cell::RefCell;
use critical_section::Mutex;
use embedded_graphics::{
    draw_target::DrawTarget,
    geometry::{Point, Size},
    pixelcolor::Rgb565,
    primitives::Rectangle,
};
use embedded_hal_bus::spi::ExclusiveDevice;
use esp_hal::{
    Blocking,
    delay::Delay,
    gpio::{Level, Output, OutputConfig},
    i2c::master::{Config as I2cConfig, I2c},
    peripherals::{GPIO3, GPIO11, GPIO12, GPIO35, GPIO36, GPIO37, I2C0, SPI2},
    spi::master::{Config as SpiConfig, Spi},
    time::Rate,
};
use slint::platform::software_renderer::{LineBufferProvider, MinimalSoftwareWindow, Rgb565Pixel};

type SpiDeviceType =
    ExclusiveDevice<Spi<'static, Blocking>, Output<'static>, embedded_hal_bus::spi::NoDelay>;

type CoreDisplay = mipidsi::Display<
    display_interface_spi::SPIInterface<SpiDeviceType, Output<'static>>,
    mipidsi::models::ILI9342CRgb565,
    mipidsi::NoResetPin,
>;

static SCREEN: Mutex<RefCell<Option<CoreDisplay>>> = Mutex::new(RefCell::new(None));

const AXP2101_ADDR: u8 = 0x34;
const AW9523_ADDR: u8 = 0x58;

fn init_pmic_and_hardware_reset(i2c: &mut impl embedded_hal::i2c::I2c, delay: &mut Delay) {
    let _ = i2c.write(AXP2101_ADDR, &[0x93, 0x1C]);
    let _ = i2c.write(AXP2101_ADDR, &[0x99, 0x1C]);

    let mut reg90 = [0u8; 1];
    if i2c.write_read(AXP2101_ADDR, &[0x90], &mut reg90).is_ok() {
        let _ = i2c.write(AXP2101_ADDR, &[0x90, reg90[0] | (1 << 1) | (1 << 7)]);
    }

    let _ = i2c.write(AW9523_ADDR, &[0x13, 0xFF]);
    let _ = i2c.write(AW9523_ADDR, &[0x05, 0x00]);

    let _ = i2c.write(AW9523_ADDR, &[0x03, 0x00]);
    delay.delay_millis(20u32);
    let _ = i2c.write(AW9523_ADDR, &[0x03, 0xFF]);
    delay.delay_millis(50u32);
}

pub fn init(
    i2c0: I2C0<'static>,
    spi2: SPI2<'static>,
    gpio12: GPIO12<'static>,
    gpio11: GPIO11<'static>,
    gpio36: GPIO36<'static>,
    gpio37: GPIO37<'static>,
    gpio35: GPIO35<'static>,
    gpio3: GPIO3<'static>,
    delay: &mut Delay,
) {
    let mut i2c = I2c::new(i2c0, I2cConfig::default())
        .unwrap()
        .with_sda(gpio12)
        .with_scl(gpio11);

    init_pmic_and_hardware_reset(&mut i2c, delay);

    let spi = Spi::new(
        spi2,
        SpiConfig::default().with_frequency(Rate::from_mhz(40)),
    )
    .unwrap()
    .with_sck(gpio36)
    .with_mosi(gpio37);

    let dc = Output::new(gpio35, Level::Low, OutputConfig::default());
    let cs = Output::new(gpio3, Level::High, OutputConfig::default());

    let spi_device =
        ExclusiveDevice::new_no_delay(spi, cs).expect("Failed to initialize SPI device");

    let di = display_interface_spi::SPIInterface::new(spi_device, dc);

    let display = mipidsi::Builder::new(mipidsi::models::ILI9342CRgb565, di)
        .color_order(mipidsi::options::ColorOrder::Bgr)
        .invert_colors(mipidsi::options::ColorInversion::Inverted)
        .orientation(
            mipidsi::options::Orientation::new(),
            // .rotate(mipidsi::options::Rotation::Deg90)
        )
        .init(delay)
        .unwrap();

    critical_section::with(|cs| {
        *SCREEN.borrow(cs).borrow_mut() = Some(display);
    });
}

// Concrete wrapper around CoreDisplay to implement Slint's LineBufferProvider
struct DisplayWrapper<'a> {
    display: &'a mut CoreDisplay,
    line_buffer: &'a mut [Rgb565Pixel; 320],
}

impl<'a> LineBufferProvider for DisplayWrapper<'a> {
    type TargetPixel = Rgb565Pixel;

    fn process_line(
        &mut self,
        line: usize,
        range: core::ops::Range<usize>,
        render_fn: impl FnOnce(&mut [Self::TargetPixel]),
    ) {
        render_fn(&mut self.line_buffer[range.clone()]);

        let _ = self.display.fill_contiguous(
            &Rectangle::new(
                Point::new(range.start as i32, line as i32),
                Size::new(range.len() as u32, 1),
            ),
            self.line_buffer[range].iter().map(|p| {
                let pixel = p.0;
                let r = ((pixel >> 11) & 0x1F) as u8;
                let g = ((pixel >> 5) & 0x3F) as u8;
                let b = (pixel & 0x1F) as u8;
                Rgb565::new(r, g, b)
            }),
        );
    }
}

pub fn render_slint_window(window: &MinimalSoftwareWindow) {
    let mut line_buffer = [Rgb565Pixel(0); 320];

    critical_section::with(|cs| {
        let mut guard = SCREEN.borrow(cs).borrow_mut();
        if let Some(display) = guard.as_mut() {
            window.draw_if_needed(|renderer| {
                renderer.render_by_line(DisplayWrapper {
                    display,
                    line_buffer: &mut line_buffer,
                });
            });
        }
    });
}
