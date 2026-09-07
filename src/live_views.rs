//! Allocation-controlled CPU0 rendering for content views.
//!
//! Continuously changing content is rasterized into one fixed PSRAM RGB565
//! framebuffer. Static page chrome uses the same buffer and embedded-graphics
//! primitives. No refresh creates or replaces heap-owned presentation objects.

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
    primitives::{PrimitiveStyle, PrimitiveStyleBuilder, Rectangle},
    text::{Baseline, Text},
};

use crate::{data_plane, models::ImuDisplay, theme, waveform};

pub const CONTENT_X: usize = 44;
pub const WIDTH: usize = 320 - CONTENT_X;
pub const HEIGHT: usize = 240;
const PIXELS: usize = WIDTH * HEIGHT;

const WHITE_RAW: u16 = theme::WHITE_RGB565;
const BLACK_RAW: u16 = theme::BLACK_RGB565;
const DARK_BLUE_RAW: u16 = theme::DARK_BLUE_RGB565;
const LIGHT_BLUE_RAW: u16 = theme::LIGHT_BLUE_RGB565;
const DARK_GRAY_RAW: u16 = theme::DARK_GRAY_RGB565;
const LIGHT_GRAY_RAW: u16 = theme::LIGHT_GRAY_RGB565;

fn color(raw: u16) -> Rgb565 {
    Rgb565::from(RawU16::new(raw))
}

pub struct Framebuffer {
    pixels: data_plane::FixedPsramBuffer<u16>,
}

impl Framebuffer {
    pub fn new() -> Self {
        Self {
            pixels: data_plane::FixedPsramBuffer::filled(PIXELS, WHITE_RAW),
        }
    }

    pub fn pixels(&self) -> &[u16] {
        self.pixels.as_slice()
    }

    fn clear_fast(&mut self, raw: u16) {
        self.pixels.as_mut_slice().fill(raw);
    }

    pub fn render_blank(&mut self) {
        self.clear_fast(WHITE_RAW);
    }

    pub fn render_placeholder(&mut self, title: &str, subtitle: &str) {
        self.clear_fast(WHITE_RAW);

        let title_style = MonoTextStyle::new(&FONT_8X13_BOLD, color(DARK_BLUE_RAW));
        let subtitle_style = MonoTextStyle::new(&FONT_6X10, color(DARK_GRAY_RAW));

        let title_x = ((WIDTH as i32 - title.len() as i32 * 8) / 2).max(8);
        let subtitle_x = ((WIDTH as i32 - subtitle.len() as i32 * 6) / 2).max(8);

        let _ = Text::with_baseline(
            title,
            Point::new(title_x, 92),
            title_style,
            Baseline::Top,
        )
        .draw(self);
        let _ = Text::with_baseline(
            subtitle,
            Point::new(subtitle_x, 114),
            subtitle_style,
            Baseline::Top,
        )
        .draw(self);
    }

    pub fn render_microphone_shell(&mut self) {
        self.clear_fast(WHITE_RAW);

        self.draw_waveform_panel(4, 4, "MIC L", waveform::LEFT_CANVAS_Y);
        self.draw_waveform_panel(4, 122, "MIC R", waveform::RIGHT_CANVAS_Y);
    }

    fn draw_waveform_panel(&mut self, x: i32, y: i32, label: &str, canvas_y: usize) {
        let panel_style = PrimitiveStyleBuilder::new()
            .fill_color(color(BLACK_RAW))
            .stroke_color(color(DARK_GRAY_RAW))
            .stroke_width(1)
            .build();
        let panel = Rectangle::new(Point::new(x, y), Size::new((WIDTH - 8) as u32, 114));
        let _ = panel.into_styled(panel_style).draw(self);

        let label_style = MonoTextStyle::new(&FONT_6X10, color(LIGHT_GRAY_RAW));
        let _ = Text::with_baseline(
            label,
            Point::new(x + 6, y + 4),
            label_style,
            Baseline::Top,
        )
        .draw(self);

        let canvas_x = waveform::CANVAS_X - CONTENT_X;
        self.fill_rect(
            canvas_x,
            canvas_y,
            waveform::CANVAS_WIDTH,
            waveform::CANVAS_HEIGHT,
            WHITE_RAW,
        );
    }

    pub fn render_log(&mut self, text: &str) {
        self.clear_fast(WHITE_RAW);

        let style = MonoTextStyle::new(&FONT_6X10, color(BLACK_RAW));
        let mut y = 4i32;
        for line in text.lines().take(23) {
            let _ = Text::with_baseline(line, Point::new(4, y), style, Baseline::Top).draw(self);
            y += 10;
        }
    }

    pub fn render_imu(&mut self, imu: &ImuDisplay) {
        self.clear_fast(WHITE_RAW);

        self.draw_header(imu);
        self.draw_attitude(imu);
        self.draw_compass(imu);
    }

    fn draw_header(&mut self, imu: &ImuDisplay) {
        let header = Rectangle::new(Point::new(6, 6), Size::new((WIDTH - 12) as u32, 44));
        let _ = header
            .into_styled(PrimitiveStyle::with_fill(color(DARK_BLUE_RAW)))
            .draw(self);

        let small = MonoTextStyle::new(&FONT_6X10, color(LIGHT_GRAY_RAW));
        let white_small = MonoTextStyle::new(&FONT_6X10, color(WHITE_RAW));
        let value = MonoTextStyle::new(&FONT_8X13_BOLD, color(WHITE_RAW));

        let _ = Text::with_baseline(
            "IMU 9-AXIS",
            Point::new(12, 10),
            white_small,
            Baseline::Top,
        )
        .draw(self);
        let _ = Text::with_baseline(
            status_text(imu.status),
            Point::new(12, 22),
            small,
            Baseline::Top,
        )
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
        let _ = Text::with_baseline(
            mag.as_str(),
            Point::new(12, 33),
            small,
            Baseline::Top,
        )
        .draw(self);

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
        let _ = Text::with_baseline(
            label,
            Point::new(x, 9),
            label_style,
            Baseline::Top,
        )
        .draw(self);

        let mut text = ArrayString::<16>::new();
        let _ = write!(&mut text, "{} deg", degrees);
        let _ = Text::with_baseline(
            text.as_str(),
            Point::new(x, 22),
            value_style,
            Baseline::Top,
        )
        .draw(self);
    }

    fn draw_attitude(&mut self, imu: &ImuDisplay) {
        const X: usize = 6;
        const Y: usize = 56;
        const W: usize = WIDTH - 12;
        const H: usize = 140;

        for local_y in 0..H {
            let row = (Y + local_y) * WIDTH + X;
            self.pixels.as_mut_slice()[row..row + W].fill(LIGHT_BLUE_RAW);
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
                self.pixels.as_mut_slice()[(Y + local_y) * WIDTH + X + local_x] = DARK_GRAY_RAW;
            }
        }

        self.hline(X, Y, W, LIGHT_GRAY_RAW);
        self.hline(X, Y + H - 1, W, LIGHT_GRAY_RAW);
        self.vline(X, Y, H, LIGHT_GRAY_RAW);
        self.vline(X + W - 1, Y, H, LIGHT_GRAY_RAW);

        let center_abs_x = X + W / 2;
        let center_abs_y = Y + H / 2;
        self.hline(center_abs_x - 36, center_abs_y, 26, WHITE_RAW);
        self.hline(center_abs_x + 10, center_abs_y, 26, WHITE_RAW);
        self.vline(center_abs_x, center_abs_y - 5, 11, WHITE_RAW);
        self.hline(center_abs_x - 20, center_abs_y - 23, 40, WHITE_RAW);
        self.hline(center_abs_x - 12, center_abs_y + 22, 24, WHITE_RAW);

        let roll_x = (center_abs_x as i32 + roll * 21 / 20 - 2)
            .clamp(X as i32, (X + W - 5) as i32) as usize;
        self.fill_rect(roll_x, Y + 5, 5, 10, WHITE_RAW);

        let white_small = MonoTextStyle::new(&FONT_6X10, color(WHITE_RAW));
        let _ = Text::with_baseline(
            "PITCH / ROLL",
            Point::new((X + 5) as i32, (Y + 4) as i32),
            white_small,
            Baseline::Top,
        )
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
                white_small,
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

        self.fill_rect(X, Y, W, H, WHITE_RAW);
        self.hline(X, Y, W, LIGHT_GRAY_RAW);
        self.hline(X, Y + H - 1, W, LIGHT_GRAY_RAW);
        self.vline(X, Y, H, LIGHT_GRAY_RAW);
        self.vline(X + W - 1, Y, H, LIGHT_GRAY_RAW);
        self.hline(X + 8, Y + 19, W - 16, LIGHT_GRAY_RAW);

        let center = (X + W / 2) as i32;
        let style_n = MonoTextStyle::new(&FONT_6X10, color(DARK_BLUE_RAW));
        let style = MonoTextStyle::new(&FONT_6X10, color(DARK_GRAY_RAW));

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

        self.fill_rect(X + W / 2 - 2, Y + 16, 4, 13, DARK_BLUE_RAW);
    }

    fn hline(&mut self, x: usize, y: usize, width: usize, raw: u16) {
        if y >= HEIGHT || x >= WIDTH {
            return;
        }
        let end = (x + width).min(WIDTH);
        self.pixels.as_mut_slice()[y * WIDTH + x..y * WIDTH + end].fill(raw);
    }

    fn vline(&mut self, x: usize, y: usize, height: usize, raw: u16) {
        if x >= WIDTH || y >= HEIGHT {
            return;
        }
        let end = (y + height).min(HEIGHT);
        for yy in y..end {
            self.pixels.as_mut_slice()[yy * WIDTH + x] = raw;
        }
    }

    fn fill_rect(&mut self, x: usize, y: usize, width: usize, height: usize, raw: u16) {
        if x >= WIDTH || y >= HEIGHT {
            return;
        }
        let x_end = (x + width).min(WIDTH);
        let y_end = (y + height).min(HEIGHT);
        for yy in y..y_end {
            self.pixels.as_mut_slice()[yy * WIDTH + x..yy * WIDTH + x_end].fill(raw);
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
