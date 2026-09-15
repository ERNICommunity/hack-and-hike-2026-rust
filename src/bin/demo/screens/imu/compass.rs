//! The compass labels of the IMU view.
//!
//! Eight labels (N, NE, E, ... NW) stand on the ground plane, on a circle
//! around the viewer (the compass ring). Each letter (glyph) is a few line
//! strokes in the 3-D world, so the labels move with the grid in
//! perspective.

use embedded_graphics::primitives::Rectangle;
use embedded_graphics::{pixelcolor::Rgb565, prelude::Point};
use hack_and_hike::ui::{Canvas, theme};

use super::projection::{
    PERSPECTIVE_NEAR_Z, PerspectiveCamera, project_camera_point, project_camera_solid_line,
    world_to_camera,
};

// The labels are on a ring with a radius of 256 world units. Each letter is a
// small sign made of lines. It stands on the ground plane and is tangent to
// the ring. When the board is level, a letter in the middle of the view is
// about 15 x 29 pixels. Near the edges of the view, the flat perspective
// projection would make such signs larger, although they are as far away as
// the others. So the size of the letters is corrected for each label. The
// position of each label stays on the ring.
/// Radius of the compass ring: the horizontal distance of the labels from
/// the viewer, in world units.
const COMPASS_LABEL_RADIUS: f32 = 256.0;
/// Width of one letter in world units, before the size correction for each
/// label.
const COMPASS_GLYPH_WIDTH: f32 = 42.0;
/// Height of one letter in world units, before the size correction for each
/// label.
const COMPASS_GLYPH_HEIGHT: f32 = 78.0;
/// Space between the letters of "NE", "SW", ..., in world units, before the
/// size correction.
const COMPASS_GLYPH_GAP: f32 = 18.0;
/// The vertical focal length, in pixels, that the letter sizes above were
/// designed for. The letters are scaled by this value divided by the camera's
/// focal length. So a change of the field of view zooms the world, but the
/// letters keep their size on the screen.
const GLYPH_REFERENCE_FOCAL_PX: f32 = 94.0;
/// Thickness of the letter strokes on screen, in pixels.
const COMPASS_STROKE_WIDTH: u32 = 3;
/// `1 / √2`: the x and z of a diagonal direction.
const INV_SQRT_2: f32 = 0.70710677;

// This alignment changes only how the labels are drawn. The fused basis is
// correct, but the compass letters were on the opposite side of the drawn
// world. So only these labels are turned by 180 degrees, so that N/S and E/W
// line up. Fusion, the magnetic correction and the camera orientation do not
// change.
/// Each label with the x and z of its direction on the ground. The label
/// stands at that unit vector times `COMPASS_LABEL_RADIUS`.
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

// Letters as line strokes `[x0, y0, x1, y1]` in a box 4 units wide and 7
// units high, with y up.
/// The strokes of the letter N.
const GLYPH_N_STROKES: [[f32; 4]; 3] = [
    [0.0, 0.0, 0.0, 7.0],
    [0.0, 7.0, 4.0, 0.0],
    [4.0, 0.0, 4.0, 7.0],
];
/// The strokes of the letter E.
const GLYPH_E_STROKES: [[f32; 4]; 4] = [
    [0.0, 0.0, 0.0, 7.0],
    [0.0, 7.0, 4.0, 7.0],
    [0.0, 3.5, 3.5, 3.5],
    [0.0, 0.0, 4.0, 0.0],
];
/// The strokes of the letter S.
const GLYPH_S_STROKES: [[f32; 4]; 5] = [
    [4.0, 7.0, 0.0, 7.0],
    [0.0, 7.0, 0.0, 4.2],
    [0.0, 4.2, 4.0, 2.8],
    [4.0, 2.8, 4.0, 0.0],
    [4.0, 0.0, 0.0, 0.0],
];
/// The strokes of the letter W.
const GLYPH_W_STROKES: [[f32; 4]; 4] = [
    [0.0, 7.0, 0.8, 0.0],
    [0.8, 0.0, 2.0, 3.2],
    [2.0, 3.2, 3.2, 0.0],
    [3.2, 0.0, 4.0, 7.0],
];

/// Draw every compass label that is in front of the camera. The labels stand
/// on the ground plane at height `ground_y`.
pub(super) fn draw_world_compass_labels(
    frame: &mut Canvas,
    area: Rectangle,
    camera: PerspectiveCamera,
    ground_y: f32,
) {
    let centered_depth = camera.sin_pitch * ground_y + camera.cos_pitch * COMPASS_LABEL_RADIUS;
    if centered_depth <= PERSPECTIVE_NEAR_Z {
        return;
    }

    // The ratio of the two focal lengths. It is 1 today, because both focal
    // lengths are equal. It keeps the letter width correct if they ever
    // differ. It does not depend on the label, so it is computed once per
    // frame.
    let glyph_horizontal_focal_scale = camera.focal_y / camera.focal_x;
    // This keeps the letter size on the screen independent of the field of
    // view.
    let glyph_zoom_scale = GLYPH_REFERENCE_FOCAL_PX / camera.focal_y;

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
        // Skip a label whose anchor is more than 64 pixels left or right of
        // the view, before any of its strokes are projected.
        if screen_x < -64 || screen_x > area.size.width as i32 + 64 {
            continue;
        }

        // `theta` is the angle between the label and the middle of the view.
        // In a flat perspective projection, a sign on the ring grows by about
        // 1 / cos(theta)^2 in width and 1 / cos(theta) in height. The depth
        // ratio below is about cos(theta), so the scales cancel that growth.
        // Only the letter size is scaled, not the anchor. So the label keeps
        // its true direction and moves with the grid, but it does not look
        // closer near the edge of the view. The ratio is clamped to the range
        // 0.2 to 1.0.
        let depth_scale = (anchor_camera[2] / centered_depth).clamp(0.2, 1.0);
        let horizontal_scale =
            depth_scale * depth_scale * glyph_horizontal_focal_scale * glyph_zoom_scale;
        let vertical_scale = depth_scale * glyph_zoom_scale;

        // The tangent points to the right of the screen when this compass
        // direction is in the middle of the view. Labels away from the middle
        // still get the real perspective distortion. Only the growth in size
        // is corrected above.
        let tangent = [unit_z, 0.0, -unit_x];
        let glyph_width = COMPASS_GLYPH_WIDTH * horizontal_scale;
        let glyph_gap = COMPASS_GLYPH_GAP * horizontal_scale;
        let glyph_count = label.len() as f32;
        let total_width = glyph_count * glyph_width + (glyph_count - 1.0).max(0.0) * glyph_gap;
        let mut placement = GlyphPlacement {
            anchor,
            tangent,
            offset: -0.5 * total_width,
            horizontal_scale,
            vertical_scale,
        };

        for glyph in label.bytes() {
            draw_world_compass_glyph(frame, area, camera, glyph, placement);
            placement.offset += glyph_width + glyph_gap;
        }
    }
}

/// Where one compass letter stands on the ground plane of the world.
#[derive(Clone, Copy)]
struct GlyphPlacement {
    /// World point at the bottom centre of the label, on the ground plane.
    anchor: [f32; 3],
    /// World direction in which the letters of the label follow each
    /// other.
    tangent: [f32; 3],
    /// Distance of this glyph's left edge from `anchor` along `tangent`.
    offset: f32,
    /// Factor for the width of this label's letters and gaps. It makes letters
    /// narrower where the projection would widen them, away from the middle
    /// of the view. It also contains the ratio of the focal lengths (1 today).
    horizontal_scale: f32,
    /// Factor for the height of this label's letters. It makes letters lower
    /// where the projection would stretch them, away from the middle of the
    /// view.
    vertical_scale: f32,
}

impl GlyphPlacement {
    /// Map stroke coordinates inside the letter box (x 0 to 4, y 0 to 7) to
    /// world coordinates.
    fn world_point(self, glyph_x: f32, glyph_y: f32) -> [f32; 3] {
        let horizontal =
            self.offset + glyph_x * (COMPASS_GLYPH_WIDTH * self.horizontal_scale / 4.0);
        [
            self.anchor[0] + self.tangent[0] * horizontal,
            self.anchor[1] + glyph_y * (COMPASS_GLYPH_HEIGHT * self.vertical_scale / 7.0),
            self.anchor[2] + self.tangent[2] * horizontal,
        ]
    }
}

/// Draw the strokes of one letter at its place on the ring.
fn draw_world_compass_glyph(
    frame: &mut Canvas,
    area: Rectangle,
    camera: PerspectiveCamera,
    glyph: u8,
    placement: GlyphPlacement,
) {
    for stroke in compass_glyph_strokes(glyph) {
        let start = placement.world_point(stroke[0], stroke[1]);
        let end = placement.world_point(stroke[2], stroke[3]);
        let start_camera = world_to_camera(start, camera);
        let end_camera = world_to_camera(end, camera);

        if let Some(line) =
            project_camera_solid_line(area, camera, start_camera, end_camera, COMPASS_STROKE_WIDTH)
        {
            draw_solid_line_pixels(
                frame,
                area,
                Point::new(line.0.0, line.0.1),
                Point::new(line.1.0, line.1.1),
                theme::WHITE,
                COMPASS_STROKE_WIDTH,
            );
        }
    }
}

/// The strokes of `glyph`. Empty for a letter that has no strokes here.
fn compass_glyph_strokes(glyph: u8) -> &'static [[f32; 4]] {
    match glyph {
        b'N' => &GLYPH_N_STROKES,
        b'E' => &GLYPH_E_STROKES,
        b'S' => &GLYPH_S_STROKES,
        b'W' => &GLYPH_W_STROKES,
        _ => &[],
    }
}

/// Draw a line from `start` to `end`, relative to `area`, with a square brush
/// `width` pixels wide. It uses the Bresenham algorithm, which steps from pixel
/// to pixel with integers only.
fn draw_solid_line_pixels(
    frame: &mut Canvas,
    area: Rectangle,
    start: Point,
    end: Point,
    color: Rgb565,
    width: u32,
) {
    let Point {
        x: mut x0,
        y: mut y0,
    } = start;
    let Point { x: x1, y: y1 } = end;
    let dx = (x1 - x0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let dy = -(y1 - y0).abs();
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut error = dx + dy;
    let half = (width as i32) / 2;
    let width_i32 = width as i32;

    loop {
        let pixel_x = area.top_left.x + x0 - half;
        let pixel_y = area.top_left.y + y0 - half;
        for offset_y in 0..width_i32 {
            for offset_x in 0..width_i32 {
                frame.set(Point::new(pixel_x + offset_x, pixel_y + offset_y), color);
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
