use core::cell::RefCell;
use critical_section::Mutex;
use embedded_graphics::{
    draw_target::DrawTarget,
    geometry::{Point, Size},
    pixelcolor::Rgb565,
    primitives::Rectangle,
};
use crate::system_i2c::SystemI2cDevice;
use embedded_hal_bus::spi::ExclusiveDevice;
use esp_hal::{
    Blocking,
    delay::Delay,
    gpio::{Level, Output, OutputConfig},
    peripherals::{GPIO3, GPIO35, GPIO36, GPIO37, SPI2},
    spi::master::{Config as SpiConfig, Spi},
    time::Rate,
};
use slint::platform::{PointerEventButton, WindowEvent};
use slint::platform::software_renderer::{LineBufferProvider, MinimalSoftwareWindow, Rgb565Pixel};
use embedded_hal::i2c::I2c;

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
const FT6336_ADDR: u8 = 0x38;

const SCREEN_WIDTH: u16 = 320;
const SCREEN_HEIGHT: u16 = 240;

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
    // P1_2 = FT6336 TOUCH_INT -> input. We poll the controller, but leave INT
    // electrically configured as an input instead of driving against it.
    update_register_bits(i2c, AW9523_ADDR, 0x05, (1 << 1) | (1 << 2), 1 << 2);

    // Reset LCD and touch controller together, while preserving the other
    // AW9523 output bits.
    update_register_bits(i2c, AW9523_ADDR, 0x03, 1 << 1, 0);
    update_register_bits(i2c, AW9523_ADDR, 0x02, 1 << 0, 0);
    delay.delay_millis(20u32);

    update_register_bits(i2c, AW9523_ADDR, 0x03, 1 << 1, 1 << 1);
    update_register_bits(i2c, AW9523_ADDR, 0x02, 1 << 0, 1 << 0);

    // FT6336U needs about 300 ms after reset before it starts reporting points.
    delay.delay_millis(300u32);
}

pub fn init(
    mut i2c: SystemI2cDevice,
    spi2: SPI2<'static>,
    gpio36: GPIO36<'static>,
    gpio37: GPIO37<'static>,
    gpio35: GPIO35<'static>,
    gpio3: GPIO3<'static>,
    delay: &mut Delay,
) -> TouchController {
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

    TouchController {
        i2c,
        state: TouchState::new(),
    }
}

#[derive(Clone, Copy)]
enum TouchSample {
    Up,
    Down { x: u16, y: u16 },
    ReadError,
}

pub struct TouchController {
    i2c: SystemI2cDevice,
    state: TouchState,
}

impl TouchController {
    fn read_sample(&mut self) -> TouchSample {
        // FT6336U registers 0x02..0x06:
        // TD_STATUS, P1_XH, P1_XL, P1_YH, P1_YL.
        let mut data = [0u8; 5];
        if self
            .i2c
            .write_read(FT6336_ADDR, &[0x02], &mut data)
            .is_err()
        {
            return TouchSample::ReadError;
        }

        let touch_count = data[0] & 0x0F;
        if touch_count == 0 {
            return TouchSample::Up;
        }

        let x = (((data[1] & 0x0F) as u16) << 8) | data[2] as u16;
        let y = (((data[3] & 0x0F) as u16) << 8) | data[4] as u16;

        if x >= SCREEN_WIDTH || y >= SCREEN_HEIGHT {
            return TouchSample::ReadError;
        }

        TouchSample::Down { x, y }
    }

    pub fn poll(&mut self, window: &MinimalSoftwareWindow) {
        let sample = self.read_sample();

        let transition = match sample {
            TouchSample::ReadError => None,
            TouchSample::Up if self.state.pressed => {
                self.state.pressed = false;
                Some(TouchTransition::Released {
                    x: self.state.x,
                    y: self.state.y,
                })
            }
            TouchSample::Up => None,
            TouchSample::Down { x, y } if !self.state.pressed => {
                self.state.pressed = true;
                self.state.x = x;
                self.state.y = y;
                Some(TouchTransition::Pressed { x, y })
            }
            TouchSample::Down { x, y } => {
                if self.state.x == x && self.state.y == y {
                    None
                } else {
                    self.state.x = x;
                    self.state.y = y;
                    Some(TouchTransition::Moved { x, y })
                }
            }
        };

        let Some(transition) = transition else {
            return;
        };

        let event = match transition {
            TouchTransition::Pressed { x, y } => WindowEvent::PointerPressed {
                position: slint::LogicalPosition {
                    x: x as f32,
                    y: y as f32,
                },
                button: PointerEventButton::Left,
            },
            TouchTransition::Moved { x, y } => WindowEvent::PointerMoved {
                position: slint::LogicalPosition {
                    x: x as f32,
                    y: y as f32,
                },
            },
            TouchTransition::Released { x, y } => WindowEvent::PointerReleased {
                position: slint::LogicalPosition {
                    x: x as f32,
                    y: y as f32,
                },
                button: PointerEventButton::Left,
            },
        };

        window.dispatch_event(event);
    }
}

#[derive(Clone, Copy)]
struct TouchState {
    pressed: bool,
    x: u16,
    y: u16,
}

impl TouchState {
    const fn new() -> Self {
        Self {
            pressed: false,
            x: 0,
            y: 0,
        }
    }
}

enum TouchTransition {
    Pressed { x: u16, y: u16 },
    Moved { x: u16, y: u16 },
    Released { x: u16, y: u16 },
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
