//! An artificial horizon with a pitch ladder, a crosshair and a compass.
//!
//! Everything is drawn with `embedded-graphics` primitives onto the canvas.

use embedded_graphics::{
    pixelcolor::Rgb565,
    prelude::*,
    primitives::{Circle, Line, PrimitiveStyle, Rectangle},
};
use hack_and_hike::{
    capabilities::imu::{Attitude, MagStatus, Sample},
    ui::{Canvas, theme},
};

const SKY: Rgb565 = theme::LIGHT_BLUE;
const GROUND: Rgb565 = theme::DARK_GRAY;

/// How far the horizon moves per degree of pitch.
const PIXELS_PER_DEGREE: f32 = 2.5;
/// Pitch ladder rungs every this many degrees, up to `LADDER_RUNGS` each way.
const LADDER_STEP_DEG: f32 = 10.0;
const LADDER_RUNGS: i32 = 3;
const LADDER_HALF_WIDTH: f32 = 22.0;
/// Long enough to cross the whole area at any angle.
const HORIZON_HALF_LENGTH: f32 = 400.0;

const CROSSHAIR_ARM: i32 = 26;
const CROSSHAIR_GAP: i32 = 10;

const COMPASS_RADIUS: i32 = 24;
const COMPASS_MARGIN: i32 = 10;

/// Draw the horizon for `sample` inside `area`.
pub(super) fn draw(canvas: &mut Canvas, area: Rectangle, sample: &Sample) {
    let attitude = sample.attitude;
    let center = area.center();

    // The horizon is a line through `origin`, running along `along`, with
    // the sky on the `up` side. Rolling right (right side down) tilts the
    // line so that its right end rises on the screen; pitching up (top edge
    // raised) moves it down, showing more sky.
    let roll = attitude.roll_deg.to_radians();
    let along = Vector::new(libm::cosf(roll), -libm::sinf(roll));
    let up = Vector::new(-libm::sinf(roll), -libm::cosf(roll));
    let origin = Vector::from(center) - up * (attitude.pitch_deg * PIXELS_PER_DEGREE);

    canvas.fill(area, SKY);
    fill_ground(canvas, area, origin, up);

    let white = |width| PrimitiveStyle::with_stroke(theme::WHITE, width);
    let Ok(()) = Line::new(
        (origin - along * HORIZON_HALF_LENGTH).point(),
        (origin + along * HORIZON_HALF_LENGTH).point(),
    )
    .into_styled(white(2))
    .draw(&mut canvas.clipped(&area));

    for rung in (-LADDER_RUNGS..=LADDER_RUNGS).filter(|&rung| rung != 0) {
        let middle = origin + up * (rung as f32 * LADDER_STEP_DEG * PIXELS_PER_DEGREE);
        let Ok(()) = Line::new(
            (middle - along * LADDER_HALF_WIDTH).point(),
            (middle + along * LADDER_HALF_WIDTH).point(),
        )
        .into_styled(white(1))
        .draw(&mut canvas.clipped(&area));
    }

    // The crosshair stays fixed to the board.
    for direction in [-1, 1] {
        let inner = center + Point::new(direction * CROSSHAIR_GAP, 0);
        let outer = center + Point::new(direction * (CROSSHAIR_GAP + CROSSHAIR_ARM), 0);
        let Ok(()) = Line::new(inner, outer).into_styled(white(2)).draw(canvas);
    }
    let Ok(()) = Circle::with_center(center, 5)
        .into_styled(PrimitiveStyle::with_fill(theme::WHITE))
        .draw(canvas);

    let compass_center = Point::new(
        area.top_left.x + area.size.width as i32 - COMPASS_MARGIN - COMPASS_RADIUS,
        area.top_left.y + area.size.height as i32 - COMPASS_MARGIN - COMPASS_RADIUS,
    );
    draw_compass(canvas, compass_center, attitude, sample.mag_status);
}

/// Colour the half of `area` below the horizon.
fn fill_ground(canvas: &mut Canvas, area: Rectangle, origin: Vector, up: Vector) {
    let left = area.top_left.x;
    let right = area.top_left.x + area.size.width as i32;
    for y in area.rows() {
        // Signed distance above the horizon, as a function of x.
        let at_left = (left as f32 - origin.x) * up.x + (y as f32 - origin.y) * up.y;
        let (from, to) = if up.x.abs() < 1e-3 {
            if at_left < 0.0 {
                (left, right)
            } else {
                continue;
            }
        } else {
            let crossing = left as f32 - at_left / up.x;
            let crossing = (crossing as i32).clamp(left, right);
            if up.x > 0.0 {
                (left, crossing)
            } else {
                (crossing, right)
            }
        };
        if from < to {
            canvas.fill(
                Rectangle::new(Point::new(from, y), Size::new((to - from) as u32, 1)),
                GROUND,
            );
        }
    }
}

/// A ring with a needle whose red half points to magnetic north.
fn draw_compass(canvas: &mut Canvas, center: Point, attitude: Attitude, status: MagStatus) {
    let ring = Circle::with_center(center, 2 * COMPASS_RADIUS as u32);
    let Ok(()) = ring
        .into_styled(PrimitiveStyle::with_fill(theme::DARK_BLUE))
        .draw(canvas);
    let Ok(()) = ring
        .into_styled(PrimitiveStyle::with_stroke(theme::WHITE, 2))
        .draw(canvas);
    if status != MagStatus::Ready {
        return;
    }

    // North sits at `heading` degrees counter-clockwise from the top edge.
    let heading = attitude.heading_deg.to_radians();
    let north = Vector::new(-libm::sinf(heading), -libm::cosf(heading));
    let tip = Vector::from(center) + north * (COMPASS_RADIUS as f32 - 5.0);
    let tail = Vector::from(center) - north * (COMPASS_RADIUS as f32 - 5.0);
    let Ok(()) = Line::new(center, tip.point())
        .into_styled(PrimitiveStyle::with_stroke(Rgb565::RED, 3))
        .draw(canvas);
    let Ok(()) = Line::new(center, tail.point())
        .into_styled(PrimitiveStyle::with_stroke(theme::WHITE, 3))
        .draw(canvas);
}

/// A 2-D vector in pixels with fractional precision.
#[derive(Clone, Copy)]
struct Vector {
    x: f32,
    y: f32,
}

impl Vector {
    const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    fn point(self) -> Point {
        Point::new(libm::roundf(self.x) as i32, libm::roundf(self.y) as i32)
    }
}

impl From<Point> for Vector {
    fn from(point: Point) -> Self {
        Self::new(point.x as f32, point.y as f32)
    }
}

impl core::ops::Add for Vector {
    type Output = Self;
    fn add(self, other: Self) -> Self {
        Self::new(self.x + other.x, self.y + other.y)
    }
}

impl core::ops::Sub for Vector {
    type Output = Self;
    fn sub(self, other: Self) -> Self {
        Self::new(self.x - other.x, self.y - other.y)
    }
}

impl core::ops::Mul<f32> for Vector {
    type Output = Self;
    fn mul(self, factor: f32) -> Self {
        Self::new(self.x * factor, self.y * factor)
    }
}
