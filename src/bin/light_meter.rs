//! A light meter: the screen shows the ambient light in lux and how close
//! something is to the front of the board, as numbers and as a bar. The
//! sensor's raw count is shown too, for calibrating the range. Cover the
//! sensor with your hand, or switch the room light off: in the dark the
//! screen turns dark too.

#![no_std]
#![no_main]

use core::fmt::Write as _;

use arrayvec::ArrayString;
use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use embedded_graphics::{
    mono_font::ascii::FONT_10X20,
    prelude::*,
    primitives::{PrimitiveStyle, Rectangle},
};
use hack_and_hike::{
    Board,
    capabilities::display::{SCREEN, SIZE},
    ui::{Canvas, common, theme},
};

esp_bootloader_esp_idf::esp_app_desc!();

/// Below this much light the screen switches to its dark colours.
const DARK_LUX: f32 = 10.0;
/// Left edge of the labels and the bar, in pixels.
const MARGIN: i32 = 20;
/// The proximity bar: full width means something at the glass.
const BAR: Rectangle = Rectangle::new(Point::new(MARGIN, 190), Size::new(280, 24));

/// What the screen shows: the sample rounded to what the text can display,
/// so the screen is only redrawn when a visible digit changes.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Shown {
    /// Whole lux.
    lux: u32,
    /// Closeness in percent.
    proximity: u8,
    /// The sensor's raw count.
    raw_proximity: u16,
}

#[esp_rtos::main]
async fn main(_spawner: Spawner) -> ! {
    let Board {
        mut display, light, ..
    } = Board::init();
    let mut canvas = Canvas::new(SIZE);

    let Some(mut light) = light else {
        canvas.clear(theme::WHITE);
        let whole_screen = canvas.bounding_box();
        common::centered_text(
            &mut canvas,
            whole_screen,
            "No light sensor found",
            common::TITLE_FONT,
            theme::CHARCOAL,
        );
        canvas.show(&mut display.surface(SCREEN));
        loop {
            Timer::after(Duration::from_secs(1)).await;
        }
    };

    let mut shown = None;
    loop {
        if let Some(sample) = light.latest() {
            let next = Shown {
                lux: sample.lux as u32,
                proximity: sample.proximity,
                raw_proximity: sample.raw_proximity,
            };
            if shown != Some(next) {
                shown = Some(next);
                draw(&mut canvas, next);
                canvas.show(&mut display.surface(SCREEN));
            }
        }

        Timer::after(Duration::from_millis(20)).await;
    }
}

/// Draw the two readings and the proximity bar.
fn draw(canvas: &mut Canvas, shown: Shown) {
    // Light colours by day, dark ones in the dark.
    let (background, text, accent) = if (shown.lux as f32) < DARK_LUX {
        (theme::CHARCOAL, theme::WHITE, theme::LIGHT_BLUE)
    } else {
        (theme::WHITE, theme::CHARCOAL, theme::DARK_BLUE)
    };
    canvas.clear(background);

    let mut value = ArrayString::<16>::new();

    common::text(
        canvas,
        "AMBIENT LIGHT",
        Point::new(MARGIN, 40),
        common::BODY_FONT,
        text,
    );
    write!(value, "{} lux", shown.lux).expect("the value fits its buffer");
    common::text(canvas, &value, Point::new(MARGIN, 60), &FONT_10X20, accent);

    common::text(
        canvas,
        "PROXIMITY",
        Point::new(MARGIN, 130),
        common::BODY_FONT,
        text,
    );
    value.clear();
    write!(value, "{} %", shown.proximity).expect("the value fits its buffer");
    common::text(canvas, &value, Point::new(MARGIN, 150), &FONT_10X20, accent);
    value.clear();
    write!(value, "raw {}", shown.raw_proximity).expect("the value fits its buffer");
    common::text(
        canvas,
        &value,
        Point::new(MARGIN, 172),
        common::BODY_FONT,
        text,
    );

    // The bar: an outline for the full range, filled as far as the count.
    let Ok(()) = BAR
        .into_styled(PrimitiveStyle::with_stroke(text, 1))
        .draw(canvas);
    let filled = BAR.size.width * u32::from(shown.proximity) / 100;
    let Ok(()) = Rectangle::new(BAR.top_left, Size::new(filled, BAR.size.height))
        .into_styled(PrimitiveStyle::with_fill(accent))
        .draw(canvas);
}
