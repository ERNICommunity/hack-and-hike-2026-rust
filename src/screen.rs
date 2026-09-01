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
    peripherals::{GPIO3, GPIO35, GPIO36, GPIO37, SPI2},
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

const AXP2101_ADDR: u8 = 0x34;
const AW9523_ADDR: u8 = 0x58;

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

/// The one and only owner of SPI2 and the LCD.
///
/// `Screen` stays on CPU0. There is no global display mutex and no other task
/// can obtain the display object.
pub struct Screen {
    display: CoreDisplay,
}

pub fn init(
    i2c: &mut impl embedded_hal::i2c::I2c,
    spi2: SPI2<'static>,
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
    .with_mosi(gpio37);

    let dc = Output::new(gpio35, Level::Low, OutputConfig::default());
    let cs = Output::new(gpio3, Level::High, OutputConfig::default());

    let spi_device =
        ExclusiveDevice::new_no_delay(spi, cs).expect("Failed to initialize SPI device");

    let di = display_interface_spi::SPIInterface::new(spi_device, dc);

    let display = mipidsi::Builder::new(mipidsi::models::ILI9342CRgb565, di)
        .color_order(mipidsi::options::ColorOrder::Bgr)
        .invert_colors(mipidsi::options::ColorInversion::Inverted)
        .orientation(mipidsi::options::Orientation::new())
        .init(delay)
        .unwrap();

    Screen { display }
}

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

impl Screen {
    /// CPU0-only Slint rendering. The display is ordinary owned state, so a
    /// blocking SPI transfer cannot hold a cross-core/global mutex.
    pub fn render_slint_window(&mut self, window: &MinimalSoftwareWindow) {
        let mut line_buffer = [Rgb565Pixel(0); 320];

        window.draw_if_needed(|renderer| {
            renderer.render_by_line(DisplayWrapper {
                display: &mut self.display,
                line_buffer: &mut line_buffer,
            });
        });
    }
}
