//! The artificial horizon: sky, ground, their perspective grids and the
//! crosshair.
//!
//! The sky and the ground are filled column by column from the horizon line.
//! The grids are lines on two horizontal planes above and below the viewer,
//! projected with the camera from `projection`; each pixel is coloured by
//! its distance from the horizon, so the lines fade into it.

use embedded_graphics::{
    pixelcolor::Rgb565,
    prelude::{Point, Size},
    primitives::Rectangle,
};
use hack_and_hike::ui::{Canvas, theme};

use super::{
    compass,
    projection::{
        DisplayAttitude, PERSPECTIVE_NEAR_Z, PerspectiveCamera, TAN_SCALE, clip_camera_near,
        clip_line, perspective_camera, project_camera_point, round_f32, world_to_camera,
    },
};

/// Below this cosine of the roll the horizon is nearly vertical, and the
/// column-by-column fill switches to a side-of-line test.
const HORIZON_VERTICAL_COS_EPSILON: f32 = 0.015;

// Crosshair in the middle of the view: a gap for the centre dot, a short
// vertical tick and two reference bars above and below.
/// Length in pixels of each horizontal arm, left and right of the centre.
const CROSSHAIR_ARM_LENGTH: u32 = 26;
/// Pixels between the centre and the inner end of each arm.
const CROSSHAIR_GAP: i32 = 10;
/// Height in pixels of the vertical tick through the centre.
const CROSSHAIR_TICK_HEIGHT: u32 = 11;
/// The bar above the centre: `(vertical offset, width)` in pixels, centred
/// horizontally. A negative offset is above the centre.
const CROSSHAIR_UPPER_BAR: (i32, u32) = (-23, 40);
/// The bar below the centre: `(vertical offset, width)` in pixels, centred
/// horizontally.
const CROSSHAIR_LOWER_BAR: (i32, u32) = (22, 24);

// Keep an 8-unit regular grid near the viewer, then progressively thin lines
// that are already sub-pixel close together. At +/-1024 world units the +/-8
// floor/ceiling planes project to less than one pixel from the horizon, so this
// is effectively the mathematical horizon at the display's resolution.
/// Grid spacing up to `GRID_NEAR_EXTENT`; doubles after each extent below.
const GRID_NEAR_SPACING: f32 = 8.0;
/// Up to this distance the lines are 8 world units apart, beyond it 16.
const GRID_NEAR_EXTENT: f32 = 96.0;
/// Up to this distance the lines are 16 world units apart, beyond it 32.
const GRID_MID_EXTENT: f32 = 192.0;
/// Up to this distance the lines are 32 world units apart, beyond it 64.
const GRID_FAR_EXTENT: f32 = 384.0;
/// How far the grid reaches in every direction, in world units.
const GRID_EXTENT: f32 = 1024.0;
/// Height of the sky plane above the viewer (and of the ground below).
const PERSPECTIVE_PLANE_HEIGHT: f32 = 8.0;

// Grid line colour by pixel distance from the horizon: lines fade towards it.
/// Index of the last entry of the fade tables: pixels this far from the
/// horizon or farther get the full line colour.
const GRID_FADE_LAST: usize = 56;
/// Sky grid colour for each pixel distance from the horizon, index 0 to
/// `GRID_FADE_LAST`: almost the sky colour at the horizon, darker further away.
const SKY_GRID_FADE: [Rgb565; 57] = [
    Rgb565::new(0, 40, 26),
    Rgb565::new(0, 40, 26),
    Rgb565::new(0, 39, 26),
    Rgb565::new(0, 39, 26),
    Rgb565::new(0, 37, 25),
    Rgb565::new(0, 37, 25),
    Rgb565::new(0, 37, 25),
    Rgb565::new(0, 34, 24),
    Rgb565::new(0, 34, 24),
    Rgb565::new(0, 34, 24),
    Rgb565::new(0, 34, 24),
    Rgb565::new(0, 31, 23),
    Rgb565::new(0, 31, 23),
    Rgb565::new(0, 31, 23),
    Rgb565::new(0, 31, 23),
    Rgb565::new(0, 31, 23),
    Rgb565::new(0, 28, 21),
    Rgb565::new(0, 28, 21),
    Rgb565::new(0, 28, 21),
    Rgb565::new(0, 28, 21),
    Rgb565::new(0, 28, 21),
    Rgb565::new(0, 28, 21),
    Rgb565::new(0, 25, 20),
    Rgb565::new(0, 25, 20),
    Rgb565::new(0, 25, 20),
    Rgb565::new(0, 25, 20),
    Rgb565::new(0, 25, 20),
    Rgb565::new(0, 25, 20),
    Rgb565::new(0, 22, 19),
    Rgb565::new(0, 22, 19),
    Rgb565::new(0, 22, 19),
    Rgb565::new(0, 22, 19),
    Rgb565::new(0, 22, 19),
    Rgb565::new(0, 22, 19),
    Rgb565::new(0, 22, 19),
    Rgb565::new(0, 22, 19),
    Rgb565::new(0, 19, 17),
    Rgb565::new(0, 19, 17),
    Rgb565::new(0, 19, 17),
    Rgb565::new(0, 19, 17),
    Rgb565::new(0, 19, 17),
    Rgb565::new(0, 19, 17),
    Rgb565::new(0, 19, 17),
    Rgb565::new(0, 19, 17),
    Rgb565::new(0, 16, 16),
    Rgb565::new(0, 16, 16),
    Rgb565::new(0, 16, 16),
    Rgb565::new(0, 16, 16),
    Rgb565::new(0, 16, 16),
    Rgb565::new(0, 16, 16),
    Rgb565::new(0, 16, 16),
    Rgb565::new(0, 16, 16),
    Rgb565::new(0, 16, 16),
    Rgb565::new(0, 16, 16),
    Rgb565::new(0, 16, 16),
    Rgb565::new(0, 16, 16),
    Rgb565::new(0, 13, 15),
];
/// Ground grid colour for each pixel distance from the horizon, index 0 to
/// `GRID_FADE_LAST`: almost the ground colour at the horizon, lighter further
/// away.
const GROUND_GRID_FADE: [Rgb565; 57] = [
    Rgb565::new(11, 23, 11),
    Rgb565::new(11, 23, 11),
    Rgb565::new(12, 24, 12),
    Rgb565::new(12, 24, 12),
    Rgb565::new(13, 25, 13),
    Rgb565::new(13, 25, 13),
    Rgb565::new(13, 25, 13),
    Rgb565::new(14, 27, 14),
    Rgb565::new(14, 27, 14),
    Rgb565::new(14, 27, 14),
    Rgb565::new(14, 27, 14),
    Rgb565::new(15, 29, 15),
    Rgb565::new(15, 29, 15),
    Rgb565::new(15, 29, 15),
    Rgb565::new(15, 29, 15),
    Rgb565::new(15, 29, 15),
    Rgb565::new(16, 31, 16),
    Rgb565::new(16, 31, 16),
    Rgb565::new(16, 31, 16),
    Rgb565::new(16, 31, 16),
    Rgb565::new(16, 31, 16),
    Rgb565::new(16, 31, 16),
    Rgb565::new(17, 33, 17),
    Rgb565::new(17, 33, 17),
    Rgb565::new(17, 33, 17),
    Rgb565::new(17, 33, 17),
    Rgb565::new(17, 33, 17),
    Rgb565::new(17, 33, 17),
    Rgb565::new(18, 35, 18),
    Rgb565::new(18, 35, 18),
    Rgb565::new(18, 35, 18),
    Rgb565::new(18, 35, 18),
    Rgb565::new(18, 35, 18),
    Rgb565::new(18, 35, 18),
    Rgb565::new(18, 35, 18),
    Rgb565::new(18, 35, 18),
    Rgb565::new(20, 38, 20),
    Rgb565::new(20, 38, 20),
    Rgb565::new(20, 38, 20),
    Rgb565::new(20, 38, 20),
    Rgb565::new(20, 38, 20),
    Rgb565::new(20, 38, 20),
    Rgb565::new(20, 38, 20),
    Rgb565::new(20, 38, 20),
    Rgb565::new(21, 41, 21),
    Rgb565::new(21, 41, 21),
    Rgb565::new(21, 41, 21),
    Rgb565::new(21, 41, 21),
    Rgb565::new(21, 41, 21),
    Rgb565::new(21, 41, 21),
    Rgb565::new(21, 41, 21),
    Rgb565::new(21, 41, 21),
    Rgb565::new(21, 41, 21),
    Rgb565::new(21, 41, 21),
    Rgb565::new(21, 41, 21),
    Rgb565::new(21, 41, 21),
    Rgb565::new(22, 44, 22),
];

/// A horizontal line `width` pixels long starting at `(x, y)`.
fn hline(frame: &mut Canvas, x: i32, y: i32, width: u32, color: Rgb565) {
    frame.fill(Rectangle::new(Point::new(x, y), Size::new(width, 1)), color);
}

/// A vertical line `height` pixels long starting at `(x, y)`.
fn vline(frame: &mut Canvas, x: i32, y: i32, height: u32, color: Rgb565) {
    frame.fill(
        Rectangle::new(Point::new(x, y), Size::new(1, height)),
        color,
    );
}

/// Which of the two grid planes a line belongs to.
#[derive(Clone, Copy)]
enum Plane {
    /// The plane above the viewer.
    Sky,
    /// The plane below the viewer, where the compass letters stand.
    Ground,
}

impl Plane {
    /// The plane's height in world units: positive is up.
    const fn height(self) -> f32 {
        match self {
            Self::Sky => PERSPECTIVE_PLANE_HEIGHT,
            Self::Ground => -PERSPECTIVE_PLANE_HEIGHT,
        }
    }

    /// Line colours by pixel distance from the horizon.
    const fn fade(self) -> &'static [Rgb565; GRID_FADE_LAST + 1] {
        match self {
            Self::Sky => &SKY_GRID_FADE,
            Self::Ground => &GROUND_GRID_FADE,
        }
    }
}

/// Draw the whole view into `area`: sky and ground, both grids with the
/// compass letters, and the crosshair on top.
pub(super) fn draw_attitude(frame: &mut Canvas, area: Rectangle, attitude: DisplayAttitude) {
    let x0 = area.top_left.x;
    let y0 = area.top_left.y;
    let width = area.size.width as i32;
    let height = area.size.height as i32;
    let center_x = width / 2;
    let center_y = height / 2;
    let camera = perspective_camera(attitude, center_x, center_y);

    frame.fill(area, theme::LIGHT_BLUE);

    if camera.cos_roll.abs() > HORIZON_VERTICAL_COS_EPSILON {
        // One reciprocal replaces a floating-point division for every column.
        let inv_cos_roll = 1.0 / camera.cos_roll;
        for local_x in 0..width {
            let x_delta = local_x - center_x;
            let horizon = round_f32(
                center_y as f32
                    + (camera.pitch_offset as f32 + camera.sin_roll * x_delta as f32)
                        * inv_cos_roll,
            )
            .clamp(0, height);

            if camera.cos_roll > 0.0 {
                if horizon < height {
                    vline(
                        frame,
                        x0 + local_x,
                        y0 + horizon,
                        (height - horizon) as u32,
                        theme::DARK_GRAY,
                    );
                }
            } else if horizon > 0 {
                vline(frame, x0 + local_x, y0, horizon as u32, theme::DARK_GRAY);
            }
        }
    } else {
        for local_x in 0..width {
            let x_delta = local_x - center_x;
            let ground_side = -camera.sin_roll * x_delta as f32 - camera.pitch_offset as f32 >= 0.0;
            if ground_side {
                vline(frame, x0 + local_x, y0, height as u32, theme::DARK_GRAY);
            }
        }
    }

    draw_perspective_world(frame, area, camera);

    let cx = x0 + center_x;
    let cy = y0 + center_y;
    let arm = CROSSHAIR_ARM_LENGTH as i32;
    hline(
        frame,
        cx - CROSSHAIR_GAP - arm,
        cy,
        CROSSHAIR_ARM_LENGTH,
        theme::WHITE,
    );
    hline(
        frame,
        cx + CROSSHAIR_GAP,
        cy,
        CROSSHAIR_ARM_LENGTH,
        theme::WHITE,
    );
    vline(
        frame,
        cx,
        cy - CROSSHAIR_TICK_HEIGHT as i32 / 2,
        CROSSHAIR_TICK_HEIGHT,
        theme::WHITE,
    );
    for (offset, width) in [CROSSHAIR_UPPER_BAR, CROSSHAIR_LOWER_BAR] {
        hline(
            frame,
            cx - width as i32 / 2,
            cy + offset,
            width,
            theme::WHITE,
        );
    }
}

/// The grids of both planes and the compass letters.
fn draw_perspective_world(frame: &mut Canvas, area: Rectangle, camera: PerspectiveCamera) {
    draw_world_grid_plane(frame, area, camera, Plane::Sky);
    draw_world_grid_plane(frame, area, camera, Plane::Ground);
    compass::draw_world_compass_labels(frame, area, camera, Plane::Ground.height());
}

/// All grid lines of one plane: dense near the viewer, sparse far away.
fn draw_world_grid_plane(
    frame: &mut Canvas,
    area: Rectangle,
    camera: PerspectiveCamera,
    plane: Plane,
) {
    draw_world_grid_coordinate(frame, area, camera, plane, 0.0);

    let mut distance = GRID_NEAR_SPACING;
    while distance <= GRID_EXTENT {
        draw_world_grid_coordinate(frame, area, camera, plane, distance);
        draw_world_grid_coordinate(frame, area, camera, plane, -distance);

        distance += if distance < GRID_NEAR_EXTENT {
            GRID_NEAR_SPACING
        } else if distance < GRID_MID_EXTENT {
            16.0
        } else if distance < GRID_FAR_EXTENT {
            32.0
        } else {
            64.0
        };
    }
}

/// The two lines of one plane at `coordinate`: one running north-south, one
/// east-west.
fn draw_world_grid_coordinate(
    frame: &mut Canvas,
    area: Rectangle,
    camera: PerspectiveCamera,
    plane: Plane,
    coordinate: f32,
) {
    let world_y = plane.height();
    draw_world_segment(
        frame,
        area,
        camera,
        [coordinate, world_y, -GRID_EXTENT],
        [coordinate, world_y, GRID_EXTENT],
        plane,
    );
    draw_world_segment(
        frame,
        area,
        camera,
        [-GRID_EXTENT, world_y, coordinate],
        [GRID_EXTENT, world_y, coordinate],
        plane,
    );
}

/// Project one world segment, clip it to the near plane and draw it.
fn draw_world_segment(
    frame: &mut Canvas,
    area: Rectangle,
    camera: PerspectiveCamera,
    start: [f32; 3],
    end: [f32; 3],
    plane: Plane,
) {
    let mut a = world_to_camera(start, camera);
    let mut b = world_to_camera(end, camera);
    if a[2] <= PERSPECTIVE_NEAR_Z && b[2] <= PERSPECTIVE_NEAR_Z {
        return;
    }

    if a[2] <= PERSPECTIVE_NEAR_Z {
        a = clip_camera_near(a, b);
    } else if b[2] <= PERSPECTIVE_NEAR_Z {
        b = clip_camera_near(b, a);
    }

    let start_screen = project_camera_point(a, camera);
    let end_screen = project_camera_point(b, camera);
    if let (Some(start_screen), Some(end_screen)) = (start_screen, end_screen) {
        draw_clipped_line(frame, area, camera, start_screen, end_screen, plane);
    }
}

/// Clip a projected line to the viewport and draw it.
fn draw_clipped_line(
    frame: &mut Canvas,
    area: Rectangle,
    camera: PerspectiveCamera,
    start: (i32, i32),
    end: (i32, i32),
    plane: Plane,
) {
    let min_x = 0;
    let max_x = area.size.width as i32 - 1;
    let min_y = 0;
    let max_y = area.size.height as i32 - 1;
    if let Some((start, end)) = clip_line(start, end, min_x, max_x, min_y, max_y) {
        draw_line_pixels(frame, area, camera, start, end, plane);
    }
}

/// Bresenham walk that carries the horizon equation along, so each pixel's
/// distance from the horizon costs two additions instead of two multiplies.
fn draw_line_pixels(
    frame: &mut Canvas,
    area: Rectangle,
    camera: PerspectiveCamera,
    (mut x0, mut y0): (i32, i32),
    (x1, y1): (i32, i32),
    plane: Plane,
) {
    let dx = (x1 - x0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let dy = -(y1 - y0).abs();
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut error = dx + dy;
    let colors = plane.fade();

    // Screen-clipped coordinates keep this comfortably inside i32 even at the
    // +/-80 degree pitch limit; no 64-bit arithmetic is needed in the hot loop.
    let mut signed_q10 =
        camera.horizon_a_q10 * x0 + camera.horizon_b_q10 * y0 + camera.horizon_c_q10;
    let step_x_q10 = camera.horizon_a_q10 * sx;
    let step_y_q10 = camera.horizon_b_q10 * sy;

    loop {
        let distance = ((signed_q10.abs() + TAN_SCALE / 2) >> 10) as usize;
        let color = colors[distance.min(GRID_FADE_LAST)];
        frame.set(
            Point::new(area.top_left.x + x0, area.top_left.y + y0),
            color,
        );

        if x0 == x1 && y0 == y1 {
            break;
        }
        let doubled = 2 * error;
        if doubled >= dy {
            error += dy;
            x0 += sx;
            signed_q10 += step_x_q10;
        }
        if doubled <= dx {
            error += dx;
            y0 += sy;
            signed_q10 += step_y_q10;
        }
    }
}
