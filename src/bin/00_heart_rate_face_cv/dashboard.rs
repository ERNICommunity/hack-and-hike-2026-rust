//! The right half of the screen: the dashboard. The FPS and the green
//! signal of the last 20 s on top, the heart rate and the spectrum of the
//! last 10 s below.
//!
//! The dashboard is drawn into its own image in PSRAM, then sent with
//! `render_from`, like the camera image. So the camera ring can be emptied
//! while the display is busy (`while_transferring`), and during drawing.
//! `Canvas::show` has no such hook: while it sent the plot, the ring
//! overflowed and frames were lost (12 instead of 20 FPS).

use core::{convert::Infallible, fmt::Write as _};

use arrayvec::ArrayString;
use embassy_time::Instant;
use embedded_graphics::{
    mono_font::MonoTextStyle,
    pixelcolor::{Rgb565, raw::RawU16},
    prelude::*,
    primitives::{Line, PrimitiveStyle, Rectangle},
    text::{Baseline, Text},
};
use hack_and_hike::{
    capabilities::{
        camera::Camera,
        display::{BYTES_PER_PIXEL, ScanlineSource, Surface},
    },
    psram,
    ui::{common, theme},
};

use crate::signal::{BPM_COUNT, HISTORY, History, MAX_BPM, MIN_BPM, Spectrum};

/// Size of the dashboard: the right half of the screen.
pub(crate) const SIZE: Size = Size::new(WIDTH as u32, HEIGHT as u32);
/// Width of the dashboard, in pixels.
const WIDTH: usize = 160;
/// Height of the dashboard, in pixels.
const HEIGHT: usize = 240;
/// Background colour of the dashboard.
const BACKGROUND: Rgb565 = theme::CHARCOAL;
/// Distance of the texts from the edges, in pixels.
const MARGIN: i32 = 2;
/// The frame of the signal plot. The curve is drawn inside it.
const PLOT: Rectangle = Rectangle::new(Point::new(MARGIN, 16), Size::new(156, 96));
/// Top of the min/max line under the plot.
const RANGE_TEXT_Y: i32 = 114;
/// Empty the camera ring after this many plotted samples.
const SAMPLES_PER_PUMP: usize = 64;
/// Empty the camera ring after clearing this many rows.
const ROWS_PER_PUMP: usize = 20;
/// Top of the status line, under the min/max line.
const STATUS_TEXT_Y: i32 = RANGE_TEXT_Y + common::DENSE_LINE_HEIGHT;
/// Top of the heart rate text, the first line of the bottom half.
const HR_TEXT_Y: i32 = 140;
/// The frame of the spectrum plot, 40 BPM on the left, 180 on the right.
const SPECTRUM_PLOT: Rectangle = Rectangle::new(Point::new(MARGIN, 156), Size::new(156, 58));
/// Top of the BPM labels under the spectrum plot.
const AXIS_TEXT_Y: i32 = 216;
/// Top of the last line: timing and dropped frames.
const FOOTER_TEXT_Y: i32 = 228;

/// The dashboard image, and the code that draws it.
pub(crate) struct Dashboard {
    /// The image in PSRAM.
    image: PixelBuffer,
}

impl Dashboard {
    /// An empty dashboard. Call once, before the loop: the memory is never
    /// freed.
    pub(crate) fn new() -> Self {
        Self {
            image: PixelBuffer {
                pixels: psram::leaked_slice(WIDTH * HEIGHT, BACKGROUND),
            },
        }
    }

    /// Draw the dashboard and send it to `surface`. `fps_tenths` is the
    /// frame rate times 10, for example 123 for 12.3 FPS. `status` goes
    /// under the signal plot. `spectrum` is `None` while there are not yet
    /// 10 s of samples. `footer` is the last line. `camera` is pumped all the
    /// time, so that no frame is lost.
    #[allow(clippy::too_many_arguments, reason = "every value the dashboard shows")]
    pub(crate) fn show(
        &mut self,
        surface: &mut Surface<'_>,
        camera: &mut Camera,
        fps_tenths: u32,
        history: &History,
        now: Instant,
        status: &str,
        spectrum: Option<&Spectrum>,
        footer: &str,
    ) {
        // Clear in slices: writing all 77 KB of PSRAM at once may take
        // longer than the camera ring holds.
        for rows in self.image.pixels.chunks_mut(WIDTH * ROWS_PER_PUMP) {
            rows.fill(BACKGROUND);
            camera.pump();
        }
        self.draw_fps(fps_tenths);
        self.draw_signal(history, now, camera);
        camera.pump();
        self.text(status, Point::new(MARGIN, STATUS_TEXT_Y), theme::WHITE);
        self.draw_spectrum(spectrum);
        self.text(footer, Point::new(MARGIN, FOOTER_TEXT_Y), theme::LIGHT_GRAY);
        camera.pump();

        surface.render_from(&mut Rows {
            pixels: &*self.image.pixels,
            camera,
        });
    }

    /// The FPS, small, in the top-right corner.
    fn draw_fps(&mut self, fps_tenths: u32) {
        let mut text = ArrayString::<16>::new();
        write!(text, "{}.{} fps", fps_tenths / 10, fps_tenths % 10)
            .expect("the text fits its buffer");
        let font = common::DENSE_FONT;
        let char_width = (font.character_size.width + font.character_spacing) as i32;
        let text_width = text.len() as i32 * char_width;
        let origin = Point::new(WIDTH as i32 - text_width - MARGIN, MARGIN);
        self.text(&text, origin, theme::WHITE);
    }

    /// The green signal as a line. The y axis runs from the lowest to the
    /// highest value, so that small changes fill the plot. The newest
    /// sample is at the right edge, a sample 20 s old at the left edge.
    fn draw_signal(&mut self, history: &History, now: Instant, camera: &mut Camera) {
        self.text("green 20 s", Point::new(MARGIN, MARGIN), theme::LIGHT_GRAY);
        let Ok(()) = PLOT
            .into_styled(PrimitiveStyle::with_stroke(theme::DARK_GRAY, 1))
            .draw(&mut self.image);

        let Some((min, max)) = history.iter().fold(None, |range, sample| {
            let (min, max) = range.unwrap_or((sample.green, sample.green));
            Some((min.min(sample.green), max.max(sample.green)))
        }) else {
            return;
        };

        let mut text = ArrayString::<32>::new();
        write!(text, "min {min:.2}  max {max:.2}").expect("the text fits its buffer");
        self.text(&text, Point::new(MARGIN, RANGE_TEXT_Y), theme::LIGHT_GRAY);

        // The curve stays 1 pixel inside the frame. A flat signal must not
        // divide by zero, so the range is at least 0.01.
        let left = PLOT.top_left.x + 1;
        let top = PLOT.top_left.y + 1;
        let width = PLOT.size.width as i32 - 2;
        let height = PLOT.size.height as i32 - 2;
        let range = (max - min).max(0.01);
        let history_ms = HISTORY.as_millis() as i32;

        let style = PrimitiveStyle::with_stroke(Rgb565::GREEN, 1);
        let mut previous: Option<Point> = None;
        for (index, sample) in history.iter().enumerate() {
            if index % SAMPLES_PER_PUMP == 0 {
                camera.pump();
            }
            let age_ms = now
                .checked_duration_since(sample.time)
                .map_or(0, |age| age.as_millis() as i32);
            let x = (left + width - 1 - age_ms * (width - 1) / history_ms).max(left);
            let y = top + ((max - sample.green) / range * (height - 1) as f32) as i32;
            let point = Point::new(x, y);
            if let Some(previous) = previous {
                let line = Line::new(previous, point).into_styled(style);
                let Ok(()) = line.draw(&mut self.image);
            }
            previous = Some(point);
        }
    }

    /// The heart rate in large letters, and the spectrum as a line: 40 BPM
    /// on the left, 180 BPM on the right, scaled to its highest value. A
    /// vertical line marks the heart rate.
    fn draw_spectrum(&mut self, spectrum: Option<&Spectrum>) {
        let mut text = ArrayString::<16>::new();
        let written = match spectrum {
            Some(spectrum) => write!(text, "HR {:.0} BPM", spectrum.bpm),
            None => write!(text, "HR: wait 10 s"),
        };
        written.expect("the text fits its buffer");
        let style = MonoTextStyle::new(common::TITLE_FONT, theme::WHITE);
        let origin = Point::new(MARGIN, HR_TEXT_Y);
        let Ok(_) = Text::with_baseline(&text, origin, style, Baseline::Top).draw(&mut self.image);

        let Ok(()) = SPECTRUM_PLOT
            .into_styled(PrimitiveStyle::with_stroke(theme::DARK_GRAY, 1))
            .draw(&mut self.image);
        // Axis labels: the lowest BPM at the left edge, the highest
        // right-aligned at the right edge.
        let mut label = ArrayString::<8>::new();
        write!(label, "{MIN_BPM}").expect("the label fits its buffer");
        self.text(&label, Point::new(MARGIN, AXIS_TEXT_Y), theme::LIGHT_GRAY);
        label.clear();
        write!(label, "{MAX_BPM}").expect("the label fits its buffer");
        let font = common::DENSE_FONT;
        let label_width =
            label.len() as i32 * (font.character_size.width + font.character_spacing) as i32;
        let right = SPECTRUM_PLOT.top_left.x + SPECTRUM_PLOT.size.width as i32;
        self.text(
            &label,
            Point::new(right - label_width, AXIS_TEXT_Y),
            theme::LIGHT_GRAY,
        );

        let Some(spectrum) = spectrum else {
            return;
        };
        let max = spectrum
            .power
            .iter()
            .fold(0.0_f32, |max, &power| max.max(power));
        if max <= 0.0 {
            return;
        }

        // The curve stays 1 pixel inside the frame.
        let left = SPECTRUM_PLOT.top_left.x + 1;
        let top = SPECTRUM_PLOT.top_left.y + 1;
        let width = SPECTRUM_PLOT.size.width as i32 - 2;
        let height = SPECTRUM_PLOT.size.height as i32 - 2;
        let last_index = (BPM_COUNT - 1) as f32;
        let x_of = |index: f32| left + (index * (width - 1) as f32 / last_index) as i32;

        let peak_x = x_of(spectrum.bpm - MIN_BPM as f32);
        let marker = Line::new(
            Point::new(peak_x, top),
            Point::new(peak_x, top + height - 1),
        );
        let Ok(()) = marker
            .into_styled(PrimitiveStyle::with_stroke(theme::LIGHT_BLUE, 1))
            .draw(&mut self.image);

        let style = PrimitiveStyle::with_stroke(Rgb565::GREEN, 1);
        let mut previous: Option<Point> = None;
        for (index, &power) in spectrum.power.iter().enumerate() {
            let y = top + height - 1 - (power / max * (height - 1) as f32) as i32;
            let point = Point::new(x_of(index as f32), y);
            if let Some(previous) = previous {
                let line = Line::new(previous, point).into_styled(style);
                let Ok(()) = line.draw(&mut self.image);
            }
            previous = Some(point);
        }
    }

    /// Draw `text` in the small font with its top-left corner at `origin`.
    /// Like `common::text`, which only draws onto a `Canvas`.
    fn text(&mut self, text: &str, origin: Point, color: Rgb565) {
        let style = MonoTextStyle::new(common::DENSE_FONT, color);
        let Ok(_) = Text::with_baseline(text, origin, style, Baseline::Top).draw(&mut self.image);
    }
}

/// An image of [`SIZE`] pixels that `embedded-graphics` can draw on.
struct PixelBuffer {
    /// The pixels, row by row.
    pixels: &'static mut [Rgb565],
}

impl OriginDimensions for PixelBuffer {
    fn size(&self) -> Size {
        SIZE
    }
}

impl DrawTarget for PixelBuffer {
    type Color = Rgb565;
    type Error = Infallible;

    /// Set each pixel. Pixels outside the image are skipped.
    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Rgb565>>,
    {
        for Pixel(point, color) in pixels {
            if let (Ok(x), Ok(y)) = (usize::try_from(point.x), usize::try_from(point.y))
                && x < WIDTH
                && y < HEIGHT
            {
                self.pixels[y * WIDTH + x] = color;
            }
        }
        Ok(())
    }

    /// Fill the whole image at once. Faster than one pixel at a time.
    fn clear(&mut self, color: Rgb565) -> Result<(), Self::Error> {
        self.pixels.fill(color);
        Ok(())
    }
}

/// The dashboard image as rows for the display. While the display sends
/// rows, the camera ring is emptied.
struct Rows<'a> {
    /// The pixels of the image, row by row.
    pixels: &'a [Rgb565],
    /// The camera to pump.
    camera: &'a mut Camera,
}

impl ScanlineSource for Rows<'_> {
    fn fill_row(&mut self, y: usize, row: &mut [u8]) {
        // Like the camera view: filling rows can be slower than sending
        // them, and then `while_transferring` does not run.
        self.camera.pump();
        let pixels = &self.pixels[y * WIDTH..(y + 1) * WIDTH];
        for (bytes, pixel) in row.chunks_exact_mut(BYTES_PER_PIXEL).zip(pixels) {
            bytes.copy_from_slice(&RawU16::from(*pixel).into_inner().to_be_bytes());
        }
    }

    fn while_transferring(&mut self) {
        self.camera.pump();
    }
}
