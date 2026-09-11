//! IMU-view compass-ring geometry and vector glyph rendering.

use embedded_graphics::{pixelcolor::Rgb565, prelude::Point};
use embedded_gui::prelude::Rect;

use super::{
    common,
    projection::{
        PERSPECTIVE_NEAR_Z, PerspectiveCamera, project_camera_point, project_camera_solid_line,
        world_to_camera,
    },
};
use crate::app::ui::gui::GuiFramebuffer;

// Compass labels stay at the original 256-unit world radius. Each glyph is a
// small vector sign standing on the ground plane and tangent to that compass
// ring. At the current 170 px attitude viewport, a centered glyph projects to
// roughly 14x26 pixels: almost exactly twice the previous 7x13 screen font.
// Off-axis rectilinear projection would otherwise magnify tangent signs even at
// a fixed radial distance, so glyph dimensions are compensated per label while
// the world anchor itself remains fixed to the same 256-unit compass ring.
const COMPASS_LABEL_RADIUS: f32 = 256.0;
const COMPASS_GLYPH_WIDTH: f32 = 42.0;
const COMPASS_GLYPH_HEIGHT: f32 = 78.0;
const COMPASS_GLYPH_GAP: f32 = 18.0;
const COMPASS_STROKE_WIDTH: u32 = 3;
const INV_SQRT_2: f32 = 0.70710677;

// Presentation-only alignment: the fused basis itself is correct, but the
// physical compass lettering is opposite to the current visual world frame.
// Rotate only these landmarks by 180 degrees so N/S and E/W line up without
// changing fusion, magnetic correction, or camera orientation.
const WORLD_COMPASS_LABELS: [(&str, f32, f32); 8] = [
    ("N", 0.0, 1.0),
    ("NE", INV_SQRT_2, INV_SQRT_2),
    ("E", 1.0, 0.0),
    ("SE", INV_SQRT_2, -INV_SQRT_2),
    ("S", 0.0, -1.0),
    ("SW", -INV_SQRT_2, -INV_SQRT_2),
    ("W", -1.0, 0.0),
    ("NW", -INV_SQRT_2, INV_SQRT_2),
];

const GLYPH_N_STROKES: [[f32; 4]; 3] = [
    [0.0, 0.0, 0.0, 7.0],
    [0.0, 7.0, 4.0, 0.0],
    [4.0, 0.0, 4.0, 7.0],
];
const GLYPH_E_STROKES: [[f32; 4]; 4] = [
    [0.0, 0.0, 0.0, 7.0],
    [0.0, 7.0, 4.0, 7.0],
    [0.0, 3.5, 3.5, 3.5],
    [0.0, 0.0, 4.0, 0.0],
];
const GLYPH_S_STROKES: [[f32; 4]; 5] = [
    [4.0, 7.0, 0.0, 7.0],
    [0.0, 7.0, 0.0, 4.2],
    [0.0, 4.2, 4.0, 2.8],
    [4.0, 2.8, 4.0, 0.0],
    [4.0, 0.0, 0.0, 0.0],
];
const GLYPH_W_STROKES: [[f32; 4]; 4] = [
    [0.0, 7.0, 0.8, 0.0],
    [0.8, 0.0, 2.0, 3.2],
    [2.0, 3.2, 3.2, 0.0],
    [3.2, 0.0, 4.0, 7.0],
];

pub(super) fn draw_world_compass_labels(
    frame: &mut GuiFramebuffer,
    area: Rect,
    camera: PerspectiveCamera,
    ground_y: f32,
) {
    let centered_depth = camera.sin_pitch * ground_y + camera.cos_pitch * COMPASS_LABEL_RADIUS;
    if centered_depth <= PERSPECTIVE_NEAR_Z {
        return;
    }

    // Keep the preferred centered glyph width after narrowing horizontal FOV.
    // Roll happens after perspective projection, so this ratio is independent
    // of roll and costs only one division per frame.
    let glyph_horizontal_focal_scale = camera.focal_y / camera.focal_x;

    for (label, unit_x, unit_z) in WORLD_COMPASS_LABELS {
        let anchor = [
            unit_x * COMPASS_LABEL_RADIUS,
            ground_y,
            unit_z * COMPASS_LABEL_RADIUS,
        ];
        let anchor_camera = world_to_camera(anchor, camera);
        if anchor_camera[2] <= PERSPECTIVE_NEAR_Z {
            continue;
        }
        let Some((screen_x, _)) = project_camera_point(anchor_camera, camera) else {
            continue;
        };
        // Cheap whole-label cull before projecting any glyph strokes.
        if screen_x < -64 || screen_x > area.w as i32 + 64 {
            continue;
        }

        // A tangent sign on a fixed-radius ring grows as roughly sec(theta)^2
        // horizontally and sec(theta) vertically under rectilinear projection.
        // Counter-scale only the glyph dimensions, not its anchor, so the label
        // keeps its true world direction and grid motion without looking closer
        // as it approaches the edge of the viewport. The focal correction keeps
        // the centered glyph width identical to the preferred baseline.
        let depth_scale = (anchor_camera[2] / centered_depth).clamp(0.2, 1.0);
        let horizontal_scale = depth_scale * depth_scale * glyph_horizontal_focal_scale;
        let vertical_scale = depth_scale;

        // Tangent points screen-right whenever this compass direction is in the
        // center of view. Off-axis labels still inherit real perspective/skew;
        // only the unwanted rectilinear size inflation is normalized above.
        let tangent = [unit_z, 0.0, -unit_x];
        let glyph_width = COMPASS_GLYPH_WIDTH * horizontal_scale;
        let glyph_gap = COMPASS_GLYPH_GAP * horizontal_scale;
        let glyph_count = label.len() as f32;
        let total_width = glyph_count * glyph_width + (glyph_count - 1.0).max(0.0) * glyph_gap;
        let mut glyph_offset = -0.5 * total_width;

        for glyph in label.bytes() {
            draw_world_compass_glyph(
                frame,
                area,
                camera,
                glyph,
                anchor,
                tangent,
                glyph_offset,
                horizontal_scale,
                vertical_scale,
            );
            glyph_offset += glyph_width + glyph_gap;
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_world_compass_glyph(
    frame: &mut GuiFramebuffer,
    area: Rect,
    camera: PerspectiveCamera,
    glyph: u8,
    anchor: [f32; 3],
    tangent: [f32; 3],
    glyph_offset: f32,
    horizontal_scale: f32,
    vertical_scale: f32,
) {
    for stroke in compass_glyph_strokes(glyph) {
        let start = compass_glyph_world_point(
            anchor,
            tangent,
            glyph_offset,
            horizontal_scale,
            vertical_scale,
            stroke[0],
            stroke[1],
        );
        let end = compass_glyph_world_point(
            anchor,
            tangent,
            glyph_offset,
            horizontal_scale,
            vertical_scale,
            stroke[2],
            stroke[3],
        );
        let start_camera = world_to_camera(start, camera);
        let end_camera = world_to_camera(end, camera);

        if let Some(line) =
            project_camera_solid_line(area, camera, start_camera, end_camera, COMPASS_STROKE_WIDTH)
        {
            draw_solid_line_pixels(
                frame,
                area,
                line.0.0,
                line.0.1,
                line.1.0,
                line.1.1,
                common::white(),
                COMPASS_STROKE_WIDTH,
            );
        }
    }
}

fn compass_glyph_strokes(glyph: u8) -> &'static [[f32; 4]] {
    match glyph {
        b'N' => &GLYPH_N_STROKES,
        b'E' => &GLYPH_E_STROKES,
        b'S' => &GLYPH_S_STROKES,
        b'W' => &GLYPH_W_STROKES,
        _ => &[],
    }
}

#[allow(clippy::too_many_arguments)]
fn compass_glyph_world_point(
    anchor: [f32; 3],
    tangent: [f32; 3],
    glyph_offset: f32,
    horizontal_scale: f32,
    vertical_scale: f32,
    glyph_x: f32,
    glyph_y: f32,
) -> [f32; 3] {
    let horizontal = glyph_offset + glyph_x * (COMPASS_GLYPH_WIDTH * horizontal_scale / 4.0);
    [
        anchor[0] + tangent[0] * horizontal,
        anchor[1] + glyph_y * (COMPASS_GLYPH_HEIGHT * vertical_scale / 7.0),
        anchor[2] + tangent[2] * horizontal,
    ]
}

fn draw_solid_line_pixels(
    frame: &mut GuiFramebuffer,
    area: Rect,
    mut x0: i32,
    mut y0: i32,
    x1: i32,
    y1: i32,
    color: Rgb565,
    width: u32,
) {
    let dx = (x1 - x0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let dy = -(y1 - y0).abs();
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut error = dx + dy;
    let half = (width as i32) / 2;
    let width_i32 = width as i32;

    loop {
        let pixel_x = area.x + x0 - half;
        let pixel_y = area.y + y0 - half;
        for offset_y in 0..width_i32 {
            for offset_x in 0..width_i32 {
                frame.set_color_at(Point::new(pixel_x + offset_x, pixel_y + offset_y), color);
            }
        }
        if x0 == x1 && y0 == y1 {
            break;
        }
        let doubled = 2 * error;
        if doubled >= dy {
            error += dy;
            x0 += sx;
        }
        if doubled <= dx {
            error += dx;
            y0 += sy;
        }
    }
}
