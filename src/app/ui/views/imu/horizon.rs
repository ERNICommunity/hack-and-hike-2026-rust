//! IMU-view artificial horizon and perspective grid rendering.

use embedded_graphics::{pixelcolor::Rgb565, prelude::Point};
use embedded_gui::prelude::Rect;

use crate::app::ui::gui::GuiFramebuffer;

use super::{
    common, compass,
    projection::{
        DisplayAttitude, PERSPECTIVE_NEAR_Z, PerspectiveCamera, TAN_SCALE, abs_f32,
        clip_camera_near, clip_line, perspective_camera, project_camera_point, round_f32,
        world_to_camera,
    },
};

const HORIZON_VERTICAL_COS_EPSILON: f32 = 0.015;

// Keep an 8-unit regular grid near the viewer, then progressively thin lines
// that are already sub-pixel close together. At +/-1024 world units the +/-8
// floor/ceiling planes project to less than one pixel from the horizon, so this
// is effectively the mathematical horizon at the display's resolution.
const GRID_NEAR_SPACING: f32 = 8.0;
const GRID_NEAR_EXTENT: f32 = 96.0;
const GRID_MID_EXTENT: f32 = 192.0;
const GRID_FAR_EXTENT: f32 = 384.0;
const GRID_EXTENT: f32 = 1024.0;
const PERSPECTIVE_PLANE_HEIGHT: f32 = 8.0;

// Exact copies of the previous fade bands indexed by rounded distance from the
// horizon. A lookup avoids the old 10-deep threshold chain for every grid pixel.
const GRID_FADE_LAST: usize = 56;
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

pub(super) fn draw_attitude(frame: &mut GuiFramebuffer, area: Rect, attitude: DisplayAttitude) {
    let x0 = area.x;
    let y0 = area.y;
    let width = area.w as i32;
    let height = area.h as i32;
    let center_x = width / 2;
    let center_y = height / 2;
    let camera = perspective_camera(attitude, center_x, center_y);

    common::fill_rect(frame, area, common::light_blue());

    if abs_f32(camera.cos_roll) > HORIZON_VERTICAL_COS_EPSILON {
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
                    common::vline(
                        frame,
                        x0 + local_x,
                        y0 + horizon,
                        (height - horizon) as u32,
                        common::dark_gray(),
                    );
                }
            } else if horizon > 0 {
                common::vline(frame, x0 + local_x, y0, horizon as u32, common::dark_gray());
            }
        }
    } else {
        for local_x in 0..width {
            let x_delta = local_x - center_x;
            let ground_side = -camera.sin_roll * x_delta as f32 - camera.pitch_offset as f32 >= 0.0;
            if ground_side {
                common::vline(frame, x0 + local_x, y0, height as u32, common::dark_gray());
            }
        }
    }

    draw_perspective_world(frame, area, camera);

    super::draw_border(frame, area);
    let cx = x0 + center_x;
    let cy = y0 + center_y;
    common::hline(frame, cx - 36, cy, 26, common::white());
    common::hline(frame, cx + 10, cy, 26, common::white());
    common::vline(frame, cx, cy - 5, 11, common::white());
    common::hline(frame, cx - 20, cy - 23, 40, common::white());
    common::hline(frame, cx - 12, cy + 22, 24, common::white());
}

fn draw_perspective_world(frame: &mut GuiFramebuffer, area: Rect, camera: PerspectiveCamera) {
    draw_world_grid_plane(frame, area, camera, PERSPECTIVE_PLANE_HEIGHT, true);
    draw_world_grid_plane(frame, area, camera, -PERSPECTIVE_PLANE_HEIGHT, false);
    compass::draw_world_compass_labels(frame, area, camera, -PERSPECTIVE_PLANE_HEIGHT);
}

fn draw_world_grid_plane(
    frame: &mut GuiFramebuffer,
    area: Rect,
    camera: PerspectiveCamera,
    world_y: f32,
    sky: bool,
) {
    draw_world_grid_coordinate(frame, area, camera, world_y, 0.0, sky);

    let mut distance = GRID_NEAR_SPACING;
    while distance <= GRID_EXTENT {
        draw_world_grid_coordinate(frame, area, camera, world_y, distance, sky);
        draw_world_grid_coordinate(frame, area, camera, world_y, -distance, sky);

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

fn draw_world_grid_coordinate(
    frame: &mut GuiFramebuffer,
    area: Rect,
    camera: PerspectiveCamera,
    world_y: f32,
    coordinate: f32,
    sky: bool,
) {
    draw_world_segment(
        frame,
        area,
        camera,
        [coordinate, world_y, -GRID_EXTENT],
        [coordinate, world_y, GRID_EXTENT],
        sky,
    );
    draw_world_segment(
        frame,
        area,
        camera,
        [-GRID_EXTENT, world_y, coordinate],
        [GRID_EXTENT, world_y, coordinate],
        sky,
    );
}

fn draw_world_segment(
    frame: &mut GuiFramebuffer,
    area: Rect,
    camera: PerspectiveCamera,
    start: [f32; 3],
    end: [f32; 3],
    sky: bool,
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
        draw_clipped_line(frame, area, camera, start_screen, end_screen, sky);
    }
}

fn draw_clipped_line(
    frame: &mut GuiFramebuffer,
    area: Rect,
    camera: PerspectiveCamera,
    start: (i32, i32),
    end: (i32, i32),
    sky: bool,
) {
    let min_x = 2;
    let max_x = area.w as i32 - 3;
    let min_y = 2;
    let max_y = area.h as i32 - 3;
    if let Some(((x0, y0), (x1, y1))) = clip_line(start, end, min_x, max_x, min_y, max_y) {
        draw_line_pixels(frame, area, camera, x0, y0, x1, y1, sky);
    }
}

/// Rasterize with the exact previous fade, but carry the horizon equation along
/// the Bresenham walk. This turns two 64-bit multiplies per pixel into small
/// 32-bit additions and writes the pixel straight to the framebuffer backend.
fn draw_line_pixels(
    frame: &mut GuiFramebuffer,
    area: Rect,
    camera: PerspectiveCamera,
    mut x0: i32,
    mut y0: i32,
    x1: i32,
    y1: i32,
    sky: bool,
) {
    let dx = (x1 - x0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let dy = -(y1 - y0).abs();
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut error = dx + dy;
    let colors = if sky {
        &SKY_GRID_FADE
    } else {
        &GROUND_GRID_FADE
    };

    // Screen-clipped coordinates keep this comfortably inside i32 even at the
    // +/-80 degree pitch limit; no 64-bit arithmetic is needed in the hot loop.
    let mut signed_q10 =
        camera.horizon_a_q10 * x0 + camera.horizon_b_q10 * y0 + camera.horizon_c_q10;
    let step_x_q10 = camera.horizon_a_q10 * sx;
    let step_y_q10 = camera.horizon_b_q10 * sy;

    loop {
        let distance = ((signed_q10.abs() + TAN_SCALE / 2) >> 10) as usize;
        let color = colors[distance.min(GRID_FADE_LAST)];
        frame.set_color_at(Point::new(area.x + x0, area.y + y0), color);

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
