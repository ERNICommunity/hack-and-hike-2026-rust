//! Allocation-free CPU0 rendering for high-rate live views.
//!
//! Slint still owns navigation and static page composition. The continuously
//! changing IMU and Log content is rasterized into one fixed PSRAM framebuffer
//! and then copied to the LCD through the existing SPI-DMA scanline pipeline.
//! No live-view refresh creates or replaces heap-owned UI objects.

use core::{convert::Infallible, fmt::Write as _};

use arrayvec::ArrayString;
use embedded_graphics::{
    draw_target::DrawTarget,
    mono_font::{
        ascii::{FONT_6X10, FONT_8X13_BOLD},
        MonoTextStyle,
    },
    pixelcolor::{raw::RawU16, Rgb565},
    prelude::*,
    primitives::{PrimitiveStyle, Rectangle},
    text::{Baseline, Text},
};

use crate::{data_plane, models::ImuDisplay, theme};

pub const CONTENT_X: usize = 44;
pub const WIDTH: usize = 320 - CONTENT_X;
pub const HEIGHT: usize = 240;
const PIXELS: usize = WIDTH * HEIGHT;

const WHITE: Rgb565 = raw_color(theme::WHITE_RGB565);
const BLACK: Rgb565 = raw_color(theme::BLACK_RGB565);
const DARK_BLUE: Rgb565 = raw_color(theme::DARK_BLUE_RGB565);
const LIGHT_BLUE: Rgb565 = raw_color(theme::LIGHT_BLUE_RGB565);
const DARK_GRAY: Rgb565 = raw_color(theme::DARK_GRAY_RGB565);
const LIGHT_GRAY: Rgb565 = raw_color(theme::LIGHT_GRAY_RGB565);

const fn raw_color(raw: u16) -> Rgb565 {
    Rgb565::from(RawU16::new(raw))
}

pub struct Framebuffer {
    pixels: data_plane::FixedPsramBuffer<u16>,
}

impl Framebuffer {
    pub fn new() -> Self {
        Self {
            pixels: data_plane::FixedPsramBuffer::filled(PIXELS, theme::WHITE_RGB565),
        }
    }

    pub fn pixels(&self) -> &[u16] {
        self.pixels.as_slice()
    }

    fn clear_fast(&mut self, color: Rgb565) {
        self.pixels.as_mut_slice().fill(color.into_storage());
    }

    pub fn render_log(&mut self, text: &str) {
        self.clear_fast(WHITE);

        let style = MonoTextStyle::new(&FONT_6X10, BLACK);
        let mut y = 4i32;
        for line in text.lines().take(23) {
            let _ = Text::with_baseline(line, Point::new(4, y), style, Baseline::Top).draw(self);
            y += 10;
        }
    }

    pub fn render_imu(&mut self, imu: &ImuDisplay) {
        self.clear_fast(WHITE);

        self.draw_header(imu);
        self.draw_attitude(imu);
        self.draw_compass(imu);
    }

    fn draw_header(&mut self, imu: &ImuDisplay) {
        let header = Rectangle::new(Point::new(6, 6), Size::new((WIDTH - 12) as u32, 44));
        let _ = header
            .into_styled(PrimitiveStyle::with_fill(DARK_BLUE))
            .draw(self);

        let small = MonoTextStyle::new(&FONT_6X10, LIGHT_GRAY);
        let white_small = MonoTextStyle::new(&FONT_6X10, WHITE);
        let value = MonoTextStyle::new(&FONT_8X13_BOLD, WHITE);

        let _ = Text::with_baseline("IMU 9-AXIS", Point::new(12, 10), white_small, Baseline::Top)
            .draw(self);
        let _ = Text::with_baseline(status_text(imu.status), Point::new(12, 22), small, Baseline::Top)
            .draw(self);

        let mut mag = ArrayString::<32>::new();
        match imu.mag_status {
            2 => {
                let _ = write!(&mut mag, "MAG {} uT", imu.mag_field_ut);
            }
            1 => {
                let _ = write!(&mut mag, "MAG CAL {}%", imu.mag_calibration);
            }
            3 => mag.push_str("MAG DISTURBED"),
            _ => mag.push_str("MAG MISSING"),
        }
        let _ = Text::with_baseline(mag.as_str(), Point::new(12, 33), small, Baseline::Top).draw(self);

        self.draw_header_value("ROLL", imu.roll_deg, 83, value, small);
        self.draw_header_value("PITCH", imu.pitch_deg, 143, value, small);
        self.draw_header_value("YAW", imu.yaw_deg, 207, value, small);
    }

    fn draw_header_value(
        &mut self,
        label: &str,
        degrees: i32,
        x: i32,
        value_style: MonoTextStyle<'static, Rgb565>,
        label_style: MonoTextStyle<'static, Rgb565>,
    ) {
        let _ = Text::with_baseline(label, Point::new(x, 9), label_style, Baseline::Top).draw(self);

        let mut text = ArrayString::<16>::new();
        let _ = write!(&mut text, "{} deg", degrees);
        let _ = Text::with_baseline(text.as_str(), Point::new(x, 22), value_style, Baseline::Top)
            .draw(self);
    }

    fn draw_attitude(&mut self, imu: &ImuDisplay) {
        const X: usize = 6;
        const Y: usize = 56;
        const W: usize = WIDTH - 12;
        const H: usize = 140;

        let sky = LIGHT_BLUE.into_storage();
        let ground = DARK_GRAY.into_storage();
        let white = WHITE.into_storage();

        for local_y in 0..H {
            let row = (Y + local_y) * WIDTH + X;
            self.pixels.as_mut_slice()[row..row + W].fill(sky);
        }

        let roll = imu.roll_deg.clamp(-45, 45);
        let pitch = imu.pitch_deg.clamp(-40, 40);
        let center_x = (W / 2) as i32;
        let center_y = (H / 2) as i32;

        for local_x in 0..W {
            let x = local_x as i32;
            let pitch_offset = pitch * 4 / 5;
            let roll_offset = roll * (x - center_x) / 300;
            let horizon = (center_y + pitch_offset + roll_offset).clamp(0, H as i32);

            for local_y in horizon as usize..H {
                self.pixels.as_mut_slice()[(Y + local_y) * WIDTH + X + local_x] = ground;
            }
        }

        // Border and fixed aircraft reference.
        self.hline(X, Y, W, theme::LIGHT_GRAY_RGB565);
        self.hline(X, Y + H - 1, W, theme::LIGHT_GRAY_RGB565);
        self.vline(X, Y, H, theme::LIGHT_GRAY_RGB565);
        self.vline(X + W - 1, Y, H, theme::LIGHT_GRAY_RGB565);

        let center_abs_x = X + W / 2;
        let center_abs_y = Y + H / 2;
        self.hline(center_abs_x - 36, center_abs_y, 26, white);
        self.hline(center_abs_x + 10, center_abs_y, 26, white);
        self.vline(center_abs_x, center_abs_y - 5, 11, white);
        self.hline(center_abs_x - 20, center_abs_y - 23, 40, white);
        self.hline(center_abs_x - 12, center_abs_y + 22, 24, white);

        let roll_x = (center_abs_x as i32 + roll * 21 / 20 - 2)
            .clamp(X as i32, (X + W - 5) as i32) as usize;
        self.fill_rect(roll_x, Y + 5, 5, 10, white);

        let white_small = MonoTextStyle::new(&FONT_6X10, WHITE);
        let _ = Text::with_baseline("PITCH / ROLL", Point::new((X + 5) as i32, (Y + 4) as i32), white_small, Baseline::Top)
            .draw(self);

        let footer = match imu.mag_status {
            1 => "Rotate through all axes - calibrating magnetometer",
            3 => "Magnetic disturbance - yaw correction paused",
            0 => "BMM150 unavailable - gyro yaw fallback",
            _ if imu.gyro_bias_ready => "Magnetic heading - gyro bias ready",
            _ => "Magnetic heading - gyro bias learning",
        };
        let _ = Text::with_baseline(
            footer,
            Point::new((X + 5) as i32, (Y + H - 13) as i32),
            white_small,
            Baseline::Top,
        )
        .draw(self);

        if imu.read_errors > 0 || imu.mag_errors > 0 {
            let mut errors = ArrayString::<28>::new();
            let _ = write!(&mut errors, "I2C {} MAG {}", imu.read_errors, imu.mag_errors);
            let _ = Text::with_baseline(
                errors.as_str(),
                Point::new((X + W - 88) as i32, (Y + 4) as i32),
                MonoTextStyle::new(&FONT_6X10, WHITE),
                Baseline::Top,
            )
            .draw(self);
        }
    }

    fn draw_compass(&mut self, imu: &ImuDisplay) {
        const X: usize = 6;
        const Y: usize = 202;
        const W: usize = WIDTH - 12;
        const H: usize = 32;

        self.fill_rect(X, Y, W, H, theme::WHITE_RGB565);
        self.hline(X, Y, W, theme::LIGHT_GRAY_RGB565);
        self.hline(X, Y + H - 1, W, theme::LIGHT_GRAY_RGB565);
        self.vline(X, Y, H, theme::LIGHT_GRAY_RGB565);
        self.vline(X + W - 1, Y, H, theme::LIGHT_GRAY_RGB565);
        self.hline(X + 8, Y + 19, W - 16, theme::LIGHT_GRAY_RGB565);

        let center = (X + W / 2) as i32;
        let style_n = MonoTextStyle::new(&FONT_6X10, DARK_BLUE);
        let style = MonoTextStyle::new(&FONT_6X10, DARK_GRAY);

        for (label, heading, label_style) in [
            ("N", 0, style_n),
            ("E", 90, style),
            ("S", 180, style),
            ("W", 270, style),
        ] {
            let delta = wrap_heading_delta(heading, imu.yaw_deg);
            let label_x = center + delta * 58 / 100 - 3;
            if label_x >= X as i32 - 6 && label_x < (X + W) as i32 {
                let _ = Text::with_baseline(
                    label,
                    Point::new(label_x, (Y + 3) as i32),
                    label_style,
                    Baseline::Top,
                )
                .draw(self);
            }
        }

        self.fill_rect(X + W / 2 - 2, Y + 16, 4, 13, theme::DARK_BLUE_RGB565);
    }

    fn hline(&mut self, x: usize, y: usize, width: usize, color: u16) {
        if y >= HEIGHT || x >= WIDTH {
            return;
        }
        let end = (x + width).min(WIDTH);
        self.pixels.as_mut_slice()[y * WIDTH + x..y * WIDTH + end].fill(color);
    }

    fn vline(&mut self, x: usize, y: usize, height: usize, color: u16) {
        if x >= WIDTH || y >= HEIGHT {
            return;
        }
        let end = (y + height).min(HEIGHT);
        for yy in y..end {
            self.pixels.as_mut_slice()[yy * WIDTH + x] = color;
        }
    }

    fn fill_rect(&mut self, x: usize, y: usize, width: usize, height: usize, color: u16) {
        if x >= WIDTH || y >= HEIGHT {
            return;
        }
        let x_end = (x + width).min(WIDTH);
        let y_end = (y + height).min(HEIGHT);
        for yy in y..y_end {
            self.pixels.as_mut_slice()[yy * WIDTH + x..yy * WIDTH + x_end].fill(color);
        }
    }
}

impl OriginDimensions for Framebuffer {
    fn size(&self) -> Size {
        Size::new(WIDTH as u32, HEIGHT as u32)
    }
}

impl DrawTarget for Framebuffer {
    type Color = Rgb565;
    type Error = Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        for Pixel(point, color) in pixels {
            if point.x < 0 || point.y < 0 {
                continue;
            }
            let x = point.x as usize;
            let y = point.y as usize;
            if x < WIDTH && y < HEIGHT {
                self.pixels.as_mut_slice()[y * WIDTH + x] = color.into_storage();
            }
        }
        Ok(())
    }

    fn fill_solid(&mut self, area: &Rectangle, color: Self::Color) -> Result<(), Self::Error> {
        let bounds = area.intersection(&self.bounding_box());
        if bounds.size == Size::zero() {
            return Ok(());
        }

        let x = bounds.top_left.x as usize;
        let y = bounds.top_left.y as usize;
        self.fill_rect(
            x,
            y,
            bounds.size.width as usize,
            bounds.size.height as usize,
            color.into_storage(),
        );
        Ok(())
    }
}

fn status_text(status: i32) -> &'static str {
    match status {
        3 => "FAULT",
        2 => "DEGRADED",
        1 => "RUNNING",
        _ => "STARTING",
    }
}

fn wrap_heading_delta(target: i32, yaw: i32) -> i32 {
    let mut delta = target - yaw;
    while delta > 180 {
        delta -= 360;
    }
    while delta < -180 {
        delta += 360;
    }
    delta
}
