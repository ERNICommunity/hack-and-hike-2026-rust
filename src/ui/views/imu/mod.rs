//! IMU attitude view with a perspective world compass.
//!
//! KDL owns the header and artificial-horizon regions. The horizon, grid and
//! distant compass labels are specialized view pixels drawn inside the attitude
//! region after layout, analogous to the direct waveform canvas on the
//! Microphone screen.

use core::fmt::Write as _;

use arrayvec::ArrayString;
use embedded_graphics::{pixelcolor::Rgb565, prelude::Point};
use embedded_gui::prelude::*;

use crate::{data_plane, display::Display, imu as sensor, models::ImuDisplay};

use super::super::gui::{GuiFramebuffer, GuiSurface};
use super::common;

mod generated {
    use embedded_gui::prelude::*;
    embedded_gui::include_gui!("src/ui/views/imu/imu.kdl");
}

const NODE_CAPACITY: usize = 8;
const TEXT_CAPACITY: usize = 4;
const EVENT_CAPACITY: usize = 2;
const VIEW_WIDTH: i32 = 276;
const VIEW_HEIGHT: i32 = 240;

const TAN_SCALE: i32 = 1024;
const TAN_STEP_DEG: i32 = 5;
const TAN_MAX_DEG: i32 = 80;
const TAN_Q10: [i32; 17] = [
    0, 90, 181, 274, 373, 477, 591, 717, 859, 1024, 1220, 1462, 1774, 2196, 2814,
    3822, 5807,
];

const PI: f32 = 3.14159265358979323846;
const RAD_TO_DEG: f32 = 180.0 / PI;
const DEG_TO_RAD: f32 = PI / 180.0;
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
const PERSPECTIVE_NEAR_Z: f32 = 0.45;

// Keep the preferred rectilinear renderer, but narrow only the horizontal field
// of view to reduce its unavoidable sec(theta)^2 yaw-speed increase toward the
// edges. tan(50deg) gives a 100deg horizontal FOV at any viewport width. Roll is
// applied after projection, preserving the preferred horizon angle exactly.
const HORIZONTAL_HALF_FOV_TAN: f32 = 1.1917536;

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
// Presentation-only 180-degree alignment: the accepted fused magnetic-yaw
// convention is intentionally left untouched. These world landmarks are the
// only place where the visual compass frame is rotated to match physical north.
const WORLD_COMPASS_LABELS: [(&str, f32, f32); 8] = [
    ("N", 0.0, -1.0),
    ("NE", -INV_SQRT_2, -INV_SQRT_2),
    ("E", -1.0, 0.0),
    ("SE", -INV_SQRT_2, INV_SQRT_2),
    ("S", 0.0, 1.0),
    ("SW", INV_SQRT_2, INV_SQRT_2),
    ("W", 1.0, 0.0),
    ("NW", INV_SQRT_2, -INV_SQRT_2),
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

// Exact copies of the previous fade bands indexed by rounded distance from the
// horizon. A lookup avoids the old 10-deep threshold chain for every grid pixel.
const GRID_FADE_LAST: usize = 56;
const SKY_GRID_FADE: [Rgb565; 57] = [
    Rgb565::new(0, 40, 26), Rgb565::new(0, 40, 26), Rgb565::new(0, 39, 26), Rgb565::new(0, 39, 26),
    Rgb565::new(0, 37, 25), Rgb565::new(0, 37, 25), Rgb565::new(0, 37, 25), Rgb565::new(0, 34, 24),
    Rgb565::new(0, 34, 24), Rgb565::new(0, 34, 24), Rgb565::new(0, 34, 24), Rgb565::new(0, 31, 23),
    Rgb565::new(0, 31, 23), Rgb565::new(0, 31, 23), Rgb565::new(0, 31, 23), Rgb565::new(0, 31, 23),
    Rgb565::new(0, 28, 21), Rgb565::new(0, 28, 21), Rgb565::new(0, 28, 21), Rgb565::new(0, 28, 21),
    Rgb565::new(0, 28, 21), Rgb565::new(0, 28, 21), Rgb565::new(0, 25, 20), Rgb565::new(0, 25, 20),
    Rgb565::new(0, 25, 20), Rgb565::new(0, 25, 20), Rgb565::new(0, 25, 20), Rgb565::new(0, 25, 20),
    Rgb565::new(0, 22, 19), Rgb565::new(0, 22, 19), Rgb565::new(0, 22, 19), Rgb565::new(0, 22, 19),
    Rgb565::new(0, 22, 19), Rgb565::new(0, 22, 19), Rgb565::new(0, 22, 19), Rgb565::new(0, 22, 19),
    Rgb565::new(0, 19, 17), Rgb565::new(0, 19, 17), Rgb565::new(0, 19, 17), Rgb565::new(0, 19, 17),
    Rgb565::new(0, 19, 17), Rgb565::new(0, 19, 17), Rgb565::new(0, 19, 17), Rgb565::new(0, 19, 17),
    Rgb565::new(0, 16, 16), Rgb565::new(0, 16, 16), Rgb565::new(0, 16, 16), Rgb565::new(0, 16, 16),
    Rgb565::new(0, 16, 16), Rgb565::new(0, 16, 16), Rgb565::new(0, 16, 16), Rgb565::new(0, 16, 16),
    Rgb565::new(0, 16, 16), Rgb565::new(0, 16, 16), Rgb565::new(0, 16, 16), Rgb565::new(0, 16, 16),
    Rgb565::new(0, 13, 15),
];
const GROUND_GRID_FADE: [Rgb565; 57] = [
    Rgb565::new(11, 23, 11), Rgb565::new(11, 23, 11), Rgb565::new(12, 24, 12), Rgb565::new(12, 24, 12),
    Rgb565::new(13, 25, 13), Rgb565::new(13, 25, 13), Rgb565::new(13, 25, 13), Rgb565::new(14, 27, 14),
    Rgb565::new(14, 27, 14), Rgb565::new(14, 27, 14), Rgb565::new(14, 27, 14), Rgb565::new(15, 29, 15),
    Rgb565::new(15, 29, 15), Rgb565::new(15, 29, 15), Rgb565::new(15, 29, 15), Rgb565::new(15, 29, 15),
    Rgb565::new(16, 31, 16), Rgb565::new(16, 31, 16), Rgb565::new(16, 31, 16), Rgb565::new(16, 31, 16),
    Rgb565::new(16, 31, 16), Rgb565::new(16, 31, 16), Rgb565::new(17, 33, 17), Rgb565::new(17, 33, 17),
    Rgb565::new(17, 33, 17), Rgb565::new(17, 33, 17), Rgb565::new(17, 33, 17), Rgb565::new(17, 33, 17),
    Rgb565::new(18, 35, 18), Rgb565::new(18, 35, 18), Rgb565::new(18, 35, 18), Rgb565::new(18, 35, 18),
    Rgb565::new(18, 35, 18), Rgb565::new(18, 35, 18), Rgb565::new(18, 35, 18), Rgb565::new(18, 35, 18),
    Rgb565::new(20, 38, 20), Rgb565::new(20, 38, 20), Rgb565::new(20, 38, 20), Rgb565::new(20, 38, 20),
    Rgb565::new(20, 38, 20), Rgb565::new(20, 38, 20), Rgb565::new(20, 38, 20), Rgb565::new(20, 38, 20),
    Rgb565::new(21, 41, 21), Rgb565::new(21, 41, 21), Rgb565::new(21, 41, 21), Rgb565::new(21, 41, 21),
    Rgb565::new(21, 41, 21), Rgb565::new(21, 41, 21), Rgb565::new(21, 41, 21), Rgb565::new(21, 41, 21),
    Rgb565::new(21, 41, 21), Rgb565::new(21, 41, 21), Rgb565::new(21, 41, 21), Rgb565::new(21, 41, 21),
    Rgb565::new(22, 44, 22),
];

type Context = GuiContext<'static, NODE_CAPACITY, TEXT_CAPACITY, EVENT_CAPACITY>;
type ScreenLine = ((i32, i32), (i32, i32));

#[derive(Clone, Copy)]
struct Geometry {
    header: Rect,
    attitude: Rect,
}

#[derive(Clone, Copy)]
struct DisplayAttitude {
    roll_deg: f32,
    pitch_deg: f32,
    yaw_deg: i32,
}

#[derive(Clone, Copy)]
struct PerspectiveCamera {
    center_x: i32,
    center_y: i32,
    focal_x: f32,
    focal_y: f32,
    sin_yaw: f32,
    cos_yaw: f32,
    sin_pitch: f32,
    cos_pitch: f32,
    sin_roll: f32,
    cos_roll: f32,
    pitch_offset: i32,
    // Q10 coefficients for the displayed horizon's implicit line:
    // a*x + b*y + c = 0. Perpendicular screen distance from this line is a
    // cheap proxy for inverse world depth on the sky/ground planes.
    horizon_a_q10: i32,
    horizon_b_q10: i32,
    horizon_c_q10: i32,
}

pub(crate) struct View {
    geometry: Geometry,
}

impl View {
    pub(crate) fn new() -> Self {
        let gui = data_plane::leaked_value_with(|| Context::new(Rect::new(0, 0, VIEW_WIDTH as u32, VIEW_HEIGHT as u32)));
        let app = generated::ImuApp::build(gui).expect("IMU KDL exceeds embedded-gui capacities");
        Self {
            geometry: Geometry {
                header: required_rect(gui, app.widgets.header_slot, "IMU header"),
                attitude: required_rect(gui, app.widgets.attitude_slot, "IMU attitude"),
            },
        }
    }

    pub(crate) fn present_shell(&mut self, surface: &mut GuiSurface, display: &mut Display) {
        let geometry = self.geometry;
        surface.present_overlay_only(display, move |frame| {
            draw_view_gutters(frame, geometry);
            draw_shell(frame, geometry);
        });
    }

    pub(crate) fn present(
        &mut self,
        surface: &mut GuiSurface,
        display: &mut Display,
        imu: &ImuDisplay,
    ) {
        let geometry = self.geometry;
        let attitude = display_attitude(imu);
        surface.present_overlay_only(display, move |frame| {
            draw_view_gutters(frame, geometry);
            draw_header(frame, geometry.header, imu, attitude);
            draw_attitude(frame, geometry.attitude, attitude);
        });
    }
}

fn draw_view_gutters(frame: &mut GuiFramebuffer, geometry: Geometry) {
    let white = common::white();
    let header_y = geometry.header.y.clamp(0, VIEW_HEIGHT);
    let header_bottom = (geometry.header.y + geometry.header.h as i32).clamp(0, VIEW_HEIGHT);
    let attitude_y = geometry.attitude.y.clamp(0, VIEW_HEIGHT);
    let attitude_bottom = (geometry.attitude.y + geometry.attitude.h as i32).clamp(0, VIEW_HEIGHT);

    fill_band(frame, 0, 0, VIEW_WIDTH, header_y, white);
    fill_band(frame, 0, header_bottom, VIEW_WIDTH, attitude_y - header_bottom, white);
    fill_band(frame, 0, attitude_bottom, VIEW_WIDTH, VIEW_HEIGHT - attitude_bottom, white);

    let header_right = (geometry.header.x + geometry.header.w as i32).clamp(0, VIEW_WIDTH);
    fill_band(frame, 0, header_y, geometry.header.x.max(0), geometry.header.h as i32, white);
    fill_band(frame, header_right, header_y, VIEW_WIDTH - header_right, geometry.header.h as i32, white);

    let attitude_right = (geometry.attitude.x + geometry.attitude.w as i32).clamp(0, VIEW_WIDTH);
    fill_band(frame, 0, attitude_y, geometry.attitude.x.max(0), geometry.attitude.h as i32, white);
    fill_band(frame, attitude_right, attitude_y, VIEW_WIDTH - attitude_right, geometry.attitude.h as i32, white);
}

fn fill_band(
    frame: &mut GuiFramebuffer,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    color: Rgb565,
) {
    if width > 0 && height > 0 {
        common::fill_box(frame, x, y, width as u32, height as u32, color);
    }
}

fn draw_shell(frame: &mut GuiFramebuffer, geometry: Geometry) {
    common::fill_rect(frame, geometry.header, common::dark_blue());
    common::draw_title(
        frame,
        "IMU",
        geometry.header.x + 6,
        geometry.header.y + 3,
        common::white(),
    );
    common::draw_body(
        frame,
        "WAITING",
        geometry.header.x + 6,
        geometry.header.y + 20,
        common::white(),
    );
    common::fill_rect(frame, geometry.attitude, common::light_blue());
    draw_border(frame, geometry.attitude);
}

fn draw_header(
    frame: &mut GuiFramebuffer,
    area: Rect,
    imu: &ImuDisplay,
    attitude: DisplayAttitude,
) {
    common::fill_rect(frame, area, common::dark_blue());
    common::draw_title(frame, "IMU", area.x + 6, area.y + 3, common::white());
    common::draw_body(
        frame,
        status_text(imu.status),
        area.x + 6,
        area.y + 19,
        common::white(),
    );

    let mut mag = ArrayString::<24>::new();
    match imu.mag_status {
        sensor::MagStatus::Ready => {
            let _ = write!(&mut mag, "MAG {}uT", imu.mag_field_ut);
        }
        sensor::MagStatus::Learning => {
            let _ = write!(&mut mag, "CAL {}%", imu.mag_calibration);
        }
        sensor::MagStatus::Disturbed => {
            let _ = write!(&mut mag, "DIST {}uT", imu.mag_field_ut);
        }
        sensor::MagStatus::Missing => mag.push_str("MAG MISSING"),
    }
    common::draw_body(
        frame,
        mag.as_str(),
        area.x + 6,
        area.y + 35,
        common::light_gray(),
    );

    let first_x = area.x + 78;
    let column_width = ((area.w as i32 - 78) / 3).max(1);
    draw_header_value(frame, "ROLL", round_degrees(attitude.roll_deg), first_x, area.y);
    draw_header_value(
        frame,
        "PITCH",
        round_degrees(attitude.pitch_deg),
        first_x + column_width,
        area.y,
    );
    draw_header_value(
        frame,
        "YAW",
        attitude.yaw_deg,
        first_x + column_width * 2,
        area.y,
    );
}

fn draw_header_value(frame: &mut GuiFramebuffer, label: &str, degrees: i32, x: i32, y: i32) {
    common::draw_body(frame, label, x, y + 3, common::white());
    let mut value = ArrayString::<12>::new();
    let _ = write!(&mut value, "{:+}", degrees);
    common::draw_title(frame, value.as_str(), x, y + 22, common::white());
}

fn draw_attitude(frame: &mut GuiFramebuffer, area: Rect, attitude: DisplayAttitude) {
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
                common::vline(
                    frame,
                    x0 + local_x,
                    y0,
                    horizon as u32,
                    common::dark_gray(),
                );
            }
        }
    } else {
        for local_x in 0..width {
            let x_delta = local_x - center_x;
            let ground_side =
                -camera.sin_roll * x_delta as f32 - camera.pitch_offset as f32 >= 0.0;
            if ground_side {
                common::vline(
                    frame,
                    x0 + local_x,
                    y0,
                    height as u32,
                    common::dark_gray(),
                );
            }
        }
    }

    draw_perspective_world(frame, area, camera);

    draw_border(frame, area);
    let cx = x0 + center_x;
    let cy = y0 + center_y;
    common::hline(frame, cx - 36, cy, 26, common::white());
    common::hline(frame, cx + 10, cy, 26, common::white());
    common::vline(frame, cx, cy - 5, 11, common::white());
    common::hline(frame, cx - 20, cy - 23, 40, common::white());
    common::hline(frame, cx - 12, cy + 22, 24, common::white());
}

fn perspective_camera(
    attitude: DisplayAttitude,
    center_x: i32,
    center_y: i32,
) -> PerspectiveCamera {
    let yaw = attitude.yaw_deg as f32 * DEG_TO_RAD;
    let pitch = attitude.pitch_deg * DEG_TO_RAD;
    let roll = attitude.roll_deg * DEG_TO_RAD;
    let sin_yaw = sin_approx(yaw);
    let cos_yaw = cos_approx(yaw);
    let sin_pitch = sin_approx(pitch);
    let cos_pitch = cos_approx(pitch);
    let sin_roll = sin_approx(roll);
    let cos_roll = cos_approx(roll);
    let (focal_x, focal_y) = perspective_focals(center_x, center_y);

    // Use the exact displayed horizon geometry for fading. This keeps the depth
    // cue attached to the attitude horizon even at high pitch/roll angles.
    let visual_pitch = round_degrees(attitude.pitch_deg).clamp(-TAN_MAX_DEG, TAN_MAX_DEG);
    let pitch_offset = project_angle(visual_pitch, center_y);
    let horizon_a_q10 = round_f32(-sin_roll * TAN_SCALE as f32);
    let horizon_b_q10 = round_f32(cos_roll * TAN_SCALE as f32);
    let horizon_c_q10 = round_f32(
        (sin_roll * center_x as f32
            - cos_roll * center_y as f32
            - pitch_offset as f32)
            * TAN_SCALE as f32,
    );

    PerspectiveCamera {
        center_x,
        center_y,
        focal_x,
        focal_y,
        sin_yaw,
        cos_yaw,
        sin_pitch,
        cos_pitch,
        sin_roll,
        cos_roll,
        pitch_offset,
        horizon_a_q10,
        horizon_b_q10,
        horizon_c_q10,
    }
}

fn draw_perspective_world(
    frame: &mut GuiFramebuffer,
    area: Rect,
    camera: PerspectiveCamera,
) {
    draw_world_grid_plane(frame, area, camera, PERSPECTIVE_PLANE_HEIGHT, true);
    draw_world_grid_plane(frame, area, camera, -PERSPECTIVE_PLANE_HEIGHT, false);
    draw_world_compass_labels(frame, area, camera);
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

fn draw_world_compass_labels(
    frame: &mut GuiFramebuffer,
    area: Rect,
    camera: PerspectiveCamera,
) {
    let centered_depth = camera.sin_pitch * -PERSPECTIVE_PLANE_HEIGHT
        + camera.cos_pitch * COMPASS_LABEL_RADIUS;
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
            -PERSPECTIVE_PLANE_HEIGHT,
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
        let horizontal_scale =
            depth_scale * depth_scale * glyph_horizontal_focal_scale;
        let vertical_scale = depth_scale;

        // Tangent points screen-right whenever this compass direction is in the
        // center of view. Off-axis labels still inherit real perspective/skew;
        // only the unwanted rectilinear size inflation is normalized above.
        let tangent = [unit_z, 0.0, -unit_x];
        let glyph_width = COMPASS_GLYPH_WIDTH * horizontal_scale;
        let glyph_gap = COMPASS_GLYPH_GAP * horizontal_scale;
        let glyph_count = label.len() as f32;
        let total_width =
            glyph_count * glyph_width + (glyph_count - 1.0).max(0.0) * glyph_gap;
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

        if let Some(line) = project_camera_solid_line(
            area,
            camera,
            start_camera,
            end_camera,
            COMPASS_STROKE_WIDTH,
        ) {
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
    let horizontal = glyph_offset
        + glyph_x * (COMPASS_GLYPH_WIDTH * horizontal_scale / 4.0);
    [
        anchor[0] + tangent[0] * horizontal,
        anchor[1] + glyph_y * (COMPASS_GLYPH_HEIGHT * vertical_scale / 7.0),
        anchor[2] + tangent[2] * horizontal,
    ]
}

fn project_camera_solid_line(
    area: Rect,
    camera: PerspectiveCamera,
    mut a: [f32; 3],
    mut b: [f32; 3],
    width: u32,
) -> Option<ScreenLine> {
    if a[2] <= PERSPECTIVE_NEAR_Z && b[2] <= PERSPECTIVE_NEAR_Z {
        return None;
    }
    if a[2] <= PERSPECTIVE_NEAR_Z {
        a = clip_camera_near(a, b);
    } else if b[2] <= PERSPECTIVE_NEAR_Z {
        b = clip_camera_near(b, a);
    }

    let start = project_camera_point(a, camera)?;
    let end = project_camera_point(b, camera)?;
    let half = (width as i32) / 2;
    clip_line(
        start,
        end,
        2 + half,
        area.w as i32 - 3 - half,
        2 + half,
        area.h as i32 - 3 - half,
    )
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
                frame.set_color_at(
                    Point::new(pixel_x + offset_x, pixel_y + offset_y),
                    color,
                );
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

fn world_to_camera(point: [f32; 3], camera: PerspectiveCamera) -> [f32; 3] {
    let yaw_x = camera.cos_yaw * point[0] - camera.sin_yaw * point[2];
    let yaw_z = camera.sin_yaw * point[0] + camera.cos_yaw * point[2];
    let pitched_y = camera.cos_pitch * point[1] - camera.sin_pitch * yaw_z;
    let pitched_z = camera.sin_pitch * point[1] + camera.cos_pitch * yaw_z;

    // Keep roll out of 3D camera space. Applying it after the anisotropic
    // perspective projection preserves the exact roll angle of the preferred
    // renderer while still allowing a narrower horizontal FOV.
    [yaw_x, pitched_y, pitched_z]
}

fn clip_camera_near(behind: [f32; 3], front: [f32; 3]) -> [f32; 3] {
    let denominator = front[2] - behind[2];
    if abs_f32(denominator) < 0.0001 {
        return [behind[0], behind[1], PERSPECTIVE_NEAR_Z];
    }
    let t = (PERSPECTIVE_NEAR_Z - behind[2]) / denominator;
    [
        behind[0] + (front[0] - behind[0]) * t,
        behind[1] + (front[1] - behind[1]) * t,
        PERSPECTIVE_NEAR_Z,
    ]
}

fn project_camera_point(point: [f32; 3], camera: PerspectiveCamera) -> Option<(i32, i32)> {
    if point[2] < PERSPECTIVE_NEAR_Z {
        return None;
    }

    let unrolled_x = camera.focal_x * point[0] / point[2];
    let unrolled_y = -camera.focal_y * point[1] / point[2];
    Some((
        round_f32(
            camera.center_x as f32
                + camera.cos_roll * unrolled_x
                - camera.sin_roll * unrolled_y,
        ),
        round_f32(
            camera.center_y as f32
                + camera.sin_roll * unrolled_x
                + camera.cos_roll * unrolled_y,
        ),
    ))
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

fn clip_line(
    mut a: (i32, i32),
    mut b: (i32, i32),
    min_x: i32,
    max_x: i32,
    min_y: i32,
    max_y: i32,
) -> Option<((i32, i32), (i32, i32))> {
    loop {
        let code_a = outcode(a.0, a.1, min_x, max_x, min_y, max_y);
        let code_b = outcode(b.0, b.1, min_x, max_x, min_y, max_y);
        if code_a | code_b == 0 {
            return Some((a, b));
        }
        if code_a & code_b != 0 {
            return None;
        }

        let code = if code_a != 0 { code_a } else { code_b };
        let dx = (b.0 - a.0) as i64;
        let dy = (b.1 - a.1) as i64;
        let (x, y) = if code & 8 != 0 {
            if dy == 0 {
                return None;
            }
            let y = max_y;
            let x = a.0 + (dx * (y - a.1) as i64 / dy) as i32;
            (x, y)
        } else if code & 4 != 0 {
            if dy == 0 {
                return None;
            }
            let y = min_y;
            let x = a.0 + (dx * (y - a.1) as i64 / dy) as i32;
            (x, y)
        } else if code & 2 != 0 {
            if dx == 0 {
                return None;
            }
            let x = max_x;
            let y = a.1 + (dy * (x - a.0) as i64 / dx) as i32;
            (x, y)
        } else {
            if dx == 0 {
                return None;
            }
            let x = min_x;
            let y = a.1 + (dy * (x - a.0) as i64 / dx) as i32;
            (x, y)
        };

        if code == code_a {
            a = (x, y);
        } else {
            b = (x, y);
        }
    }
}

fn outcode(x: i32, y: i32, min_x: i32, max_x: i32, min_y: i32, max_y: i32) -> u8 {
    let mut code = 0;
    if x < min_x {
        code |= 1;
    }
    if x > max_x {
        code |= 2;
    }
    if y < min_y {
        code |= 4;
    }
    if y > max_y {
        code |= 8;
    }
    code
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
    let mut signed_q10 = camera.horizon_a_q10 * x0
        + camera.horizon_b_q10 * y0
        + camera.horizon_c_q10;
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

fn display_attitude(imu: &ImuDisplay) -> DisplayAttitude {
    let sensor_roll = imu.roll_deg as f32 * DEG_TO_RAD;
    let sensor_pitch = imu.pitch_deg as f32 * DEG_TO_RAD;
    let sin_sensor_roll = sin_approx(sensor_roll);
    let cos_sensor_roll = cos_approx(sensor_roll);
    let sin_sensor_pitch = sin_approx(sensor_pitch);
    let cos_sensor_pitch = cos_approx(sensor_pitch);

    let ax = -sin_sensor_pitch;
    let ay = sin_sensor_roll * cos_sensor_pitch;
    let az = cos_sensor_roll * cos_sensor_pitch;

    let screen_roll = atan2_approx(-ax, -ay) * RAD_TO_DEG;
    let horizontal = sqrt_approx(ax * ax + ay * ay);
    let screen_pitch = -atan2_approx(az, horizontal) * RAD_TO_DEG;
    DisplayAttitude {
        roll_deg: screen_roll,
        pitch_deg: screen_pitch,
        yaw_deg: display_yaw(imu),
    }
}

/// The fused yaw convention is opposite to the physical left/right direction
/// desired by the screen instruments. Flip it only at the presentation boundary
/// so fusion math and magnetic correction keep a single internal convention.
fn display_yaw(imu: &ImuDisplay) -> i32 {
    -imu.yaw_deg
}

fn perspective_focals(center_x: i32, center_y: i32) -> (f32, f32) {
    let focal_x = center_x.max(1) as f32 / HORIZONTAL_HALF_FOV_TAN;
    let focal_y = center_y.max(1) as f32;
    (focal_x, focal_y)
}

fn project_angle(degrees: i32, focal_pixels: i32) -> i32 {
    focal_pixels * tangent_q10(degrees) / TAN_SCALE
}

fn tangent_q10(degrees: i32) -> i32 {
    let clamped = degrees.clamp(-TAN_MAX_DEG, TAN_MAX_DEG);
    let (sign, magnitude) = if clamped < 0 {
        (-1, -clamped)
    } else {
        (1, clamped)
    };

    let lower_index = (magnitude / TAN_STEP_DEG) as usize;
    if lower_index >= TAN_Q10.len() - 1 {
        return sign * TAN_Q10[TAN_Q10.len() - 1];
    }

    let remainder = magnitude % TAN_STEP_DEG;
    let lower = TAN_Q10[lower_index];
    let upper = TAN_Q10[lower_index + 1];
    sign * (lower + (upper - lower) * remainder / TAN_STEP_DEG)
}

fn round_degrees(value: f32) -> i32 {
    round_f32(value)
}

fn round_f32(value: f32) -> i32 {
    if value >= 0.0 {
        (value + 0.5) as i32
    } else {
        (value - 0.5) as i32
    }
}

fn abs_f32(value: f32) -> f32 {
    if value < 0.0 { -value } else { value }
}

fn wrap_radians(mut value: f32) -> f32 {
    while value > PI {
        value -= 2.0 * PI;
    }
    while value < -PI {
        value += 2.0 * PI;
    }
    value
}

fn sqrt_approx(value: f32) -> f32 {
    if value <= 0.0 {
        return 0.0;
    }

    let mut estimate = if value > 1.0 { value } else { 1.0 };
    for _ in 0..6 {
        estimate = 0.5 * (estimate + value / estimate);
    }
    estimate
}

fn atan2_approx(y: f32, x: f32) -> f32 {
    if x == 0.0 && y == 0.0 {
        return 0.0;
    }

    let abs_y = abs_f32(y) + 1.0e-10;
    let (ratio, base) = if x < 0.0 {
        ((x + abs_y) / (abs_y - x), 3.0 * PI / 4.0)
    } else {
        ((x - abs_y) / (x + abs_y), PI / 4.0)
    };
    let angle = base + (0.1963 * ratio * ratio - 0.9817) * ratio;

    if y < 0.0 { -angle } else { angle }
}

fn sin_approx(value: f32) -> f32 {
    let mut x = wrap_radians(value);
    if x > PI * 0.5 {
        x = PI - x;
    } else if x < -PI * 0.5 {
        x = -PI - x;
    }

    let x2 = x * x;
    x * (1.0 - x2 / 6.0 + x2 * x2 / 120.0 - x2 * x2 * x2 / 5040.0)
}

fn cos_approx(value: f32) -> f32 {
    sin_approx(value + PI * 0.5)
}

fn draw_border(frame: &mut GuiFramebuffer, area: Rect) {
    let width = area.w as u32;
    let height = area.h as u32;
    common::hline(frame, area.x, area.y, width, common::light_gray());
    common::hline(
        frame,
        area.x,
        area.y + area.h as i32 - 1,
        width,
        common::light_gray(),
    );
    common::vline(frame, area.x, area.y, height, common::light_gray());
    common::vline(
        frame,
        area.x + area.w as i32 - 1,
        area.y,
        height,
        common::light_gray(),
    );
}

fn status_text(status: sensor::Status) -> &'static str {
    match status {
        sensor::Status::Starting => "STARTING",
        sensor::Status::Running => "RUNNING",
        sensor::Status::Degraded => "DEGRADED",
        sensor::Status::Fault => "FAULT",
    }
}

fn required_rect(gui: &Context, id: WidgetId, name: &'static str) -> Rect {
    gui.absolute_rect(id).unwrap_or_else(|| panic!("{name} layout missing"))
}
