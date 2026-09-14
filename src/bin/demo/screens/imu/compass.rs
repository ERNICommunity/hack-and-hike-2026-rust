//! IMU-view compass-ring geometry and vector glyph rendering.

use embedded_graphics::primitives::Rectangle;
use embedded_graphics::{pixelcolor::Rgb565, prelude::Point};
use hack_and_hike::ui::{Canvas, theme};

use super::projection::{
    PERSPECTIVE_NEAR_Z, PerspectiveCamera, project_camera_point, project_camera_solid_line,
    world_to_camera,
};

// Compass labels stay at the original 256-unit world radius. Each glyph is a
// small vector sign standing on the ground plane and tangent to that compass
// ring. At the current 170 px attitude viewport, a centered glyph projects to
// roughly 14x26 pixels: almost exactly twice the previous 7x13 screen font.
// Off-axis rectilinear projection would otherwise magnify tangent signs even at
// a fixed radial distance, so glyph dimensions are compensated per label while
// the world anchor itself remains fixed to the same 256-unit compass ring.
/// Distance of the letters from the viewer, in world units.
const COMPASS_LABEL_RADIUS: f32 = 256.0;
/// Size of one letter in world units before the per-label compensation.
const COMPASS_GLYPH_WIDTH: f32 = 42.0;
/// Height of one letter in world units before the per-label compensation.
const COMPASS_GLYPH_HEIGHT: f32 = 78.0;
/// Space between the letters of "NE", "SW", ...
const COMPASS_GLYPH_GAP: f32 = 18.0;
/// The vertical focal length the letter sizes above were designed for, in
/// pixels. The letters are scaled by this over the camera's focal length, so
/// changing the field of view zooms the world but keeps the letters readable
/// at the same size.
const GLYPH_REFERENCE_FOCAL_PX: f32 = 94.0;
/// Thickness of the letter strokes on screen, in pixels.
const COMPASS_STROKE_WIDTH: u32 = 3;
/// `1 / √2`: the x and z of a diagonal direction.
const INV_SQRT_2: f32 = 0.70710677;

// Presentation-only alignment: the fused basis itself is correct, but the
// physical compass lettering is opposite to the current visual world frame.
// Rotate only these landmarks by 180 degrees so N/S and E/W line up without
// changing fusion, magnetic correction, or camera orientation.
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

// Letters as line strokes `[x0, y0, x1, y1]` in a 4 x 7 box, y up.
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

/// Draw every compass label that is in front of the camera, standing on the
/// ground plane at `ground_y`.
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

    // Keep the preferred centered glyph width after narrowing horizontal FOV.
    // Roll happens after perspective projection, so this ratio is independent
    // of roll and costs only one division per frame.
    let glyph_horizontal_focal_scale = camera.focal_y / camera.focal_x;
    // Keep the on-screen letter size independent of the field of view.
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
        // Cheap whole-label cull before projecting any glyph strokes.
        if screen_x < -64 || screen_x > area.size.width as i32 + 64 {
            continue;
        }

        // A tangent sign on a fixed-radius ring grows as roughly sec(theta)^2
        // horizontally and sec(theta) vertically under rectilinear projection.
        // Counter-scale only the glyph dimensions, not its anchor, so the label
        // keeps its true world direction and grid motion without looking closer
        // as it approaches the edge of the viewport. The focal correction keeps
        // the centered glyph width identical to the preferred baseline.
        let depth_scale = (anchor_camera[2] / centered_depth).clamp(0.2, 1.0);
        let horizontal_scale =
            depth_scale * depth_scale * glyph_horizontal_focal_scale * glyph_zoom_scale;
        let vertical_scale = depth_scale * glyph_zoom_scale;

        // Tangent points screen-right whenever this compass direction is in the
        // center of view. Off-axis labels still inherit real perspective/skew;
        // only the unwanted rectilinear size inflation is normalized above.
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

/// Where one compass glyph sits on the world ground plane.
#[derive(Clone, Copy)]
struct GlyphPlacement {
    /// World point at the center of the label.
    anchor: [f32; 3],
    /// World direction along which glyphs of the label are laid out.
    tangent: [f32; 3],
    /// Distance of this glyph's left edge from `anchor` along `tangent`.
    offset: f32,
    /// Factor for the width of this label's glyphs and gaps. It shrinks letters
    /// that the projection would widen off-axis, and corrects for the different
    /// horizontal and vertical focal lengths.
    horizontal_scale: f32,
    /// Factor for the height of this label's glyphs. It shrinks letters that the
    /// projection would stretch off-axis.
    vertical_scale: f32,
}

impl GlyphPlacement {
    /// Map glyph-local stroke coordinates (0..4 wide, 0..7 tall) to the world.
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

/// The strokes of `glyph`; empty for letters without a shape.
fn compass_glyph_strokes(glyph: u8) -> &'static [[f32; 4]] {
    match glyph {
        b'N' => &GLYPH_N_STROKES,
        b'E' => &GLYPH_E_STROKES,
        b'S' => &GLYPH_S_STROKES,
        b'W' => &GLYPH_W_STROKES,
        _ => &[],
    }
}

/// Bresenham's line from `start` to `end`, relative to `area`, with a square
/// brush `width` pixels wide.
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
