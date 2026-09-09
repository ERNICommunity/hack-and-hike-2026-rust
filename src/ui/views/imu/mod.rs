//! IMU attitude view with a cylindrical perspective grid and compass landmarks.
//!
//! KDL owns the header and artificial-horizon regions. The horizon, grid and
//! distant compass labels are specialized view pixels drawn inside the attitude
//! region after layout, analogous to the direct waveform canvas on the
//! Microphone screen.

use core::fmt::Write as _;

use arrayvec::ArrayString;
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
// that are already sub-pixel close together. Coordinate lines are treated as
// effectively infinite; the 1024-unit limit controls which parallel lines are
// present, while their cylindrical projection naturally converges to the
// horizon at infinite radial distance.
const GRID_NEAR_SPACING: f32 = 8.0;
const GRID_NEAR_EXTENT: f32 = 96.0;
const GRID_MID_EXTENT: f32 = 192.0;
const GRID_FAR_EXTENT: f32 = 384.0;
const GRID_EXTENT: f32 = 1024.0;
const PERSPECTIVE_PLANE_HEIGHT: f32 = 8.0;

// The old rectilinear camera used x=f*tan(theta), making yaw velocity grow as
// sec(theta)^2 toward the sides of the display. The world renderer now samples
// the horizontal view cylindrically: x=s*theta. Eight-pixel angular chords are
// visually sub-pixel accurate for these grid curves while keeping the embedded
// hot path small. The span covers the attitude viewport's rotated diagonal.
const CYLINDRICAL_SAMPLE_STEP_PX: i32 = 8;
const CYLINDRICAL_SAMPLE_MARGIN_PX: i32 = 8;
const CYLINDRICAL_MAX_HALF_SPAN_PX: i32 = 192;
const CYLINDRICAL_MAX_RAYS: usize = 49;
const CYLINDRICAL_MIN_RANGE: f32 = 1.0;

// Compass labels are real landmarks on visible far-grid intersections. Their
// anchors use the exact same cylindrical world projection as the grid, so they
// cannot drift relative to those intersections. Only the glyph shape is a
// camera-facing billboard, keeping the text readable without perspective shear.
const COMPASS_LANDMARK_EXTENT: f32 = 256.0;
const COMPASS_GLYPH_WIDTH_PX: f32 = 14.0;
const COMPASS_GLYPH_HEIGHT_PX: f32 = 26.0;
const COMPASS_GLYPH_GAP_PX: f32 = 6.0;
const COMPASS_CULL_MARGIN_PX: i32 = 40;
const COMPASS_STROKE_WIDTH: u32 = 3;
const WORLD_COMPASS_LANDMARKS: [(&str, [f32; 3]); 8] = [
    (
        "N",
        [0.0, -PERSPECTIVE_PLANE_HEIGHT, COMPASS_LANDMARK_EXTENT],
    ),
    (
        "NE",
        [
            COMPASS_LANDMARK_EXTENT,
            -PERSPECTIVE_PLANE_HEIGHT,
            COMPASS_LANDMARK_EXTENT,
        ],
    ),
    (
        "E",
        [COMPASS_LANDMARK_EXTENT, -PERSPECTIVE_PLANE_HEIGHT, 0.0],
    ),
    (
        "SE",
        [
            COMPASS_LANDMARK_EXTENT,
            -PERSPECTIVE_PLANE_HEIGHT,
            -COMPASS_LANDMARK_EXTENT,
        ],
    ),
    (
        "S",
        [0.0, -PERSPECTIVE_PLANE_HEIGHT, -COMPASS_LANDMARK_EXTENT],
    ),
    (
        "SW",
        [
            -COMPASS_LANDMARK_EXTENT,
            -PERSPECTIVE_PLANE_HEIGHT,
            -COMPASS_LANDMARK_EXTENT,
        ],
    ),
    (
        "W",
        [-COMPASS_LANDMARK_EXTENT, -PERSPECTIVE_PLANE_HEIGHT, 0.0],
    ),
    (
        "NW",
        [
            -COMPASS_LANDMARK_EXTENT,
            -PERSPECTIVE_PLANE_HEIGHT,
            COMPASS_LANDMARK_EXTENT,
        ],
    ),
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

type Context = GuiContext<'static, NODE_CAPACITY, TEXT_CAPACITY, EVENT_CAPACITY>;

#[derive(Clone, Copy)]
struct Geometry {
    header: Rect,
    attitude: Rect,
}

#[derive(Clone, Copy)]
struct CylindricalRay {
    u: f32,
    world_x: f32,
    world_z: f32,
}

#[derive(Clone, Copy)]
struct PerspectiveCamera {
    center_x: i32,
    center_y: i32,
    vertical_focal: f32,
    horizontal_scale: f32,
    yaw_radians: f32,
    pitch_offset: f32,
    sin_roll: f32,
    cos_roll: f32,
    // Q10 coefficients for the displayed horizon's implicit line:
    // a*x + b*y + c = 0. Perpendicular screen distance from this line is a
    // cheap proxy for inverse world depth on the sky/ground planes.
    horizon_a_q10: i32,
    horizon_b_q10: i32,
    horizon_c_q10: i32,
}

pub(crate) struct View {
    gui: &'static mut Context,
    geometry: Geometry,
}

impl View {
    pub(crate) fn new() -> Self {
        let gui = data_plane::leaked_value_with(|| Context::new(Rect::new(0, 0, 276, 240)));
        let app = generated::ImuApp::build(gui).expect("IMU KDL exceeds embedded-gui capacities");
        Self {
            geometry: Geometry {
                header: required_rect(gui, app.widgets.header_slot, "IMU header"),
                attitude: required_rect(gui, app.widgets.attitude_slot, "IMU attitude"),
            },
            gui,
        }
    }

    pub(crate) fn present_shell(&mut self, surface: &mut GuiSurface, display: &mut Display) {
        let geometry = self.geometry;
        surface.present_with_overlay(display, self.gui, move |frame| {
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
        surface.present_with_overlay(display, self.gui, move |frame| {
            draw_header(frame, geometry.header, imu);
            draw_attitude(frame, geometry.attitude, imu);
        });
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

fn draw_header(frame: &mut GuiFramebuffer, area: Rect, imu: &ImuDisplay) {
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

    let (roll_deg, pitch_deg) = display_roll_pitch(imu);
    let yaw_deg = display_yaw(imu);
    let first_x = area.x + 78;
    let column_width = ((area.w as i32 - 78) / 3).max(1);
    draw_header_value(frame, "ROLL", roll_deg, first_x, area.y);
    draw_header_value(frame, "PITCH", pitch_deg, first_x + column_width, area.y);
    draw_header_value(
        frame,
        "YAW",
        yaw_deg,
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

fn draw_attitude(frame: &mut GuiFramebuffer, area: Rect, imu: &ImuDisplay) {
    let x0 = area.x;
    let y0 = area.y;
    let width = area.w as i32;
    let height = area.h as i32;
    common::fill_rect(frame, area, common::light_blue());

    let (display_roll, display_pitch) = display_roll_pitch_f32(imu);
    let yaw_deg = display_yaw(imu);
    let pitch = round_degrees(display_pitch).clamp(-TAN_MAX_DEG, TAN_MAX_DEG);
    let center_x = width / 2;
    let center_y = height / 2;
    let pitch_offset = project_angle(pitch, center_y);
    let roll_radians = display_roll * DEG_TO_RAD;
    let sin_roll = sin_approx(roll_radians);
    let cos_roll = cos_approx(roll_radians);

    for local_x in 0..width {
        let x_delta = local_x - center_x;
        if abs_f32(cos_roll) > HORIZON_VERTICAL_COS_EPSILON {
            let horizon = round_f32(
                center_y as f32
                    + (pitch_offset as f32 + sin_roll * x_delta as f32) / cos_roll,
            )
            .clamp(0, height);

            if cos_roll > 0.0 {
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
        } else {
            let ground_side = -sin_roll * x_delta as f32 - pitch_offset as f32 >= 0.0;
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

    draw_world_view(
        frame,
        area,
        yaw_deg,
        display_roll,
        display_pitch,
        center_x,
        center_y,
    );

    draw_border(frame, area);
    let cx = x0 + center_x;
    let cy = y0 + center_y;
    common::hline(frame, cx - 36, cy, 26, common::white());
    common::hline(frame, cx + 10, cy, 26, common::white());
    common::vline(frame, cx, cy - 5, 11, common::white());
    common::hline(frame, cx - 20, cy - 23, 40, common::white());
    common::hline(frame, cx - 12, cy + 22, 24, common::white());
}

fn draw_world_view(
    frame: &mut GuiFramebuffer,
    area: Rect,
    yaw_deg: i32,
    roll_deg: f32,
    pitch_deg: f32,
    center_x: i32,
    center_y: i32,
) {
    let yaw_radians = yaw_deg as f32 * DEG_TO_RAD;
    let roll = roll_deg * DEG_TO_RAD;
    let sin_roll = sin_approx(roll);
    let cos_roll = cos_approx(roll);

    // Preserve the previous rectilinear camera's horizontal field of view while
    // redistributing it uniformly in angle. This speeds up the old slow center
    // and slows down the old fast edges without changing what bearings fit in
    // the window.
    let vertical_focal = center_y.max(1) as f32;
    let half_width = center_x.max(1) as f32;
    let rectilinear_half_fov = atan2_approx(half_width, vertical_focal).max(0.01);
    let horizontal_scale = half_width / rectilinear_half_fov;

    // Keep the artificial horizon's existing pitch response exactly. Cylindrical
    // depth is measured relative to this horizon, then the complete world view is
    // rolled in screen space.
    let visual_pitch = round_degrees(pitch_deg).clamp(-TAN_MAX_DEG, TAN_MAX_DEG);
    let pitch_offset = project_angle(visual_pitch, center_y) as f32;
    let horizon_a_q10 = round_f32(-sin_roll * TAN_SCALE as f32);
    let horizon_b_q10 = round_f32(cos_roll * TAN_SCALE as f32);
    let horizon_c_q10 = round_f32(
        (sin_roll * center_x as f32
            - cos_roll * center_y as f32
            - pitch_offset)
            * TAN_SCALE as f32,
    );

    let camera = PerspectiveCamera {
        center_x,
        center_y,
        vertical_focal,
        horizontal_scale,
        yaw_radians,
        pitch_offset,
        sin_roll,
        cos_roll,
        horizon_a_q10,
        horizon_b_q10,
        horizon_c_q10,
    };

    let (rays, ray_count) = build_cylindrical_rays(area, camera);
    let rays = &rays[..ray_count];
    draw_world_grid_plane(
        frame,
        area,
        camera,
        rays,
        PERSPECTIVE_PLANE_HEIGHT,
        true,
    );
    draw_world_grid_plane(
        frame,
        area,
        camera,
        rays,
        -PERSPECTIVE_PLANE_HEIGHT,
        false,
    );
    draw_world_compass_landmarks(frame, area, camera);
}

fn build_cylindrical_rays(
    area: Rect,
    camera: PerspectiveCamera,
) -> ([CylindricalRay; CYLINDRICAL_MAX_RAYS], usize) {
    let empty = CylindricalRay {
        u: 0.0,
        world_x: 0.0,
        world_z: 1.0,
    };
    let mut rays = [empty; CYLINDRICAL_MAX_RAYS];

    let half_width = area.w as f32 * 0.5;
    let half_height = area.h as f32 * 0.5;
    let diagonal = round_f32(sqrt_approx(
        half_width * half_width + half_height * half_height,
    )) + CYLINDRICAL_SAMPLE_MARGIN_PX;
    let mut half_span = ((diagonal + CYLINDRICAL_SAMPLE_STEP_PX - 1)
        / CYLINDRICAL_SAMPLE_STEP_PX)
        * CYLINDRICAL_SAMPLE_STEP_PX;
    half_span = half_span.min(CYLINDRICAL_MAX_HALF_SPAN_PX);

    let mut count = 0usize;
    let mut u = -half_span;
    while u <= half_span && count < CYLINDRICAL_MAX_RAYS {
        let world_bearing = camera.yaw_radians + u as f32 / camera.horizontal_scale;
        rays[count] = CylindricalRay {
            u: u as f32,
            world_x: sin_approx(world_bearing),
            world_z: cos_approx(world_bearing),
        };
        count += 1;
        u += CYLINDRICAL_SAMPLE_STEP_PX;
    }

    (rays, count)
}

fn draw_world_grid_plane(
    frame: &mut GuiFramebuffer,
    area: Rect,
    camera: PerspectiveCamera,
    rays: &[CylindricalRay],
    world_y: f32,
    sky: bool,
) {
    draw_world_grid_coordinate(frame, area, camera, rays, world_y, 0.0, sky);

    let mut distance = GRID_NEAR_SPACING;
    while distance <= GRID_EXTENT {
        draw_world_grid_coordinate(frame, area, camera, rays, world_y, distance, sky);
        draw_world_grid_coordinate(frame, area, camera, rays, world_y, -distance, sky);

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
    rays: &[CylindricalRay],
    world_y: f32,
    coordinate: f32,
    sky: bool,
) {
    if coordinate == 0.0 {
        draw_cylindrical_grid_axes(frame, area, camera, world_y, sky);
        return;
    }

    draw_cylindrical_grid_line(frame, area, camera, rays, world_y, coordinate, true, sky);
    draw_cylindrical_grid_line(
        frame,
        area,
        camera,
        rays,
        world_y,
        coordinate,
        false,
        sky,
    );
}

#[allow(clippy::too_many_arguments)]
fn draw_cylindrical_grid_line(
    frame: &mut GuiFramebuffer,
    area: Rect,
    camera: PerspectiveCamera,
    rays: &[CylindricalRay],
    world_y: f32,
    coordinate: f32,
    constant_x: bool,
    sky: bool,
) {
    // For a ray with world component d, its intersection with x=c or z=c is at
    // radial range r=c/d. Therefore y/r = y*d/c: after one division per grid
    // line, every angular sample is only multiply-adds plus clipping.
    let vertical_factor = -camera.vertical_focal * world_y / coordinate;
    let mut previous = None;

    for ray in rays {
        let component = if constant_x {
            ray.world_x
        } else {
            ray.world_z
        };
        if coordinate * component <= 0.0 {
            previous = None;
            continue;
        }

        let v = camera.pitch_offset + vertical_factor * component;
        let point = cylindrical_screen_point(ray.u, v, camera);
        if let Some(start) = previous {
            draw_clipped_line(frame, area, camera, start, point, sky);
        }
        previous = Some(point);
    }
}

fn draw_cylindrical_grid_axes(
    frame: &mut GuiFramebuffer,
    area: Rect,
    camera: PerspectiveCamera,
    world_y: f32,
    sky: bool,
) {
    // x=0 and z=0 are four radial half-axes. In cylindrical projection each is
    // a straight depth ray, so two endpoints are exact; no angular tessellation
    // is required.
    for bearing in [0.0, PI * 0.5, PI, -PI * 0.5] {
        let u = camera.horizontal_scale * wrap_radians(bearing - camera.yaw_radians);
        let near_v = camera.pitch_offset
            - camera.vertical_focal * world_y / CYLINDRICAL_MIN_RANGE;
        let far_v = camera.pitch_offset - camera.vertical_focal * world_y / GRID_EXTENT;
        let near = cylindrical_screen_point(u, near_v, camera);
        let far = cylindrical_screen_point(u, far_v, camera);
        draw_clipped_line(frame, area, camera, near, far, sky);
    }
}

fn draw_world_compass_landmarks(
    frame: &mut GuiFramebuffer,
    area: Rect,
    camera: PerspectiveCamera,
) {
    // Billboard axes are screen-space but rotate with the horizon. Only the
    // glyph shape uses this basis; each anchor itself is a real grid point.
    let tangent_x = camera.cos_roll;
    let tangent_y = camera.sin_roll;
    let up_x = camera.sin_roll;
    let up_y = -camera.cos_roll;

    for (label, anchor_world) in WORLD_COMPASS_LANDMARKS {
        let Some((anchor_x, anchor_y)) = project_cylindrical_world_point(anchor_world, camera)
        else {
            continue;
        };

        if anchor_x < -COMPASS_CULL_MARGIN_PX
            || anchor_x > area.w as i32 + COMPASS_CULL_MARGIN_PX
            || anchor_y < -COMPASS_CULL_MARGIN_PX
            || anchor_y > area.h as i32 + COMPASS_CULL_MARGIN_PX
        {
            continue;
        }

        let glyph_count = label.len() as f32;
        let total_width = glyph_count * COMPASS_GLYPH_WIDTH_PX
            + (glyph_count - 1.0).max(0.0) * COMPASS_GLYPH_GAP_PX;
        let mut glyph_offset = -0.5 * total_width;

        for glyph in label.bytes() {
            draw_compass_billboard_glyph(
                frame,
                area,
                glyph,
                anchor_x as f32,
                anchor_y as f32,
                tangent_x,
                tangent_y,
                up_x,
                up_y,
                glyph_offset,
            );
            glyph_offset += COMPASS_GLYPH_WIDTH_PX + COMPASS_GLYPH_GAP_PX;
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_compass_billboard_glyph(
    frame: &mut GuiFramebuffer,
    area: Rect,
    glyph: u8,
    anchor_x: f32,
    anchor_y: f32,
    tangent_x: f32,
    tangent_y: f32,
    up_x: f32,
    up_y: f32,
    glyph_offset: f32,
) {
    let half = (COMPASS_STROKE_WIDTH as i32) / 2;
    let min_x = 2 + half;
    let max_x = area.w as i32 - 3 - half;
    let min_y = 2 + half;
    let max_y = area.h as i32 - 3 - half;

    for stroke in compass_glyph_strokes(glyph) {
        let start_horizontal =
            glyph_offset + stroke[0] * (COMPASS_GLYPH_WIDTH_PX / 4.0);
        let start_vertical = stroke[1] * (COMPASS_GLYPH_HEIGHT_PX / 7.0);
        let end_horizontal =
            glyph_offset + stroke[2] * (COMPASS_GLYPH_WIDTH_PX / 4.0);
        let end_vertical = stroke[3] * (COMPASS_GLYPH_HEIGHT_PX / 7.0);

        let start = (
            round_f32(anchor_x + tangent_x * start_horizontal + up_x * start_vertical),
            round_f32(anchor_y + tangent_y * start_horizontal + up_y * start_vertical),
        );
        let end = (
            round_f32(anchor_x + tangent_x * end_horizontal + up_x * end_vertical),
            round_f32(anchor_y + tangent_y * end_horizontal + up_y * end_vertical),
        );

        if let Some(((x0, y0), (x1, y1))) =
            clip_line(start, end, min_x, max_x, min_y, max_y)
        {
            draw_solid_line_pixels(
                frame,
                area,
                x0,
                y0,
                x1,
                y1,
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

fn draw_solid_line_pixels(
    frame: &mut GuiFramebuffer,
    area: Rect,
    mut x0: i32,
    mut y0: i32,
    x1: i32,
    y1: i32,
    color: embedded_graphics::pixelcolor::Rgb565,
    width: u32,
) {
    let dx = (x1 - x0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let dy = -(y1 - y0).abs();
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut error = dx + dy;
    let half = (width as i32) / 2;

    loop {
        common::fill_box(
            frame,
            area.x + x0 - half,
            area.y + y0 - half,
            width,
            width,
            color,
        );
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

fn project_cylindrical_world_point(
    point: [f32; 3],
    camera: PerspectiveCamera,
) -> Option<(i32, i32)> {
    let range_sq = point[0] * point[0] + point[2] * point[2];
    if range_sq < CYLINDRICAL_MIN_RANGE * CYLINDRICAL_MIN_RANGE {
        return None;
    }

    let radial_range = sqrt_approx(range_sq);
    let world_bearing = atan2_approx(point[0], point[2]);
    let u = camera.horizontal_scale * wrap_radians(world_bearing - camera.yaw_radians);
    let v = camera.pitch_offset - camera.vertical_focal * point[1] / radial_range;
    Some(cylindrical_screen_point(u, v, camera))
}

fn cylindrical_screen_point(u: f32, v: f32, camera: PerspectiveCamera) -> (i32, i32) {
    // Roll is deliberately a final 2D rotation. This keeps the cylindrical
    // horizon straight and makes its line identical to the background horizon.
    (
        round_f32(camera.center_x as f32 + camera.cos_roll * u - camera.sin_roll * v),
        round_f32(camera.center_y as f32 + camera.sin_roll * u + camera.cos_roll * v),
    )
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

/// Rasterize with a long perspective fade toward the horizon. Distance is
/// measured from the displayed horizon in integer Q10 pixels. Cylindrical grid
/// lines remain continuous all the way to the vanishing line; only their RGB565
/// contrast is reduced, so there is no empty band around the horizon.
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

    loop {
        let distance = horizon_distance_pixels(camera, x0, y0);
        let color = grid_pixel_color(sky, distance);
        common::fill_box(frame, area.x + x0, area.y + y0, 1, 1, color);
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

fn horizon_distance_pixels(camera: PerspectiveCamera, x: i32, y: i32) -> i32 {
    let signed_q10 = camera.horizon_a_q10 as i64 * x as i64
        + camera.horizon_b_q10 as i64 * y as i64
        + camera.horizon_c_q10 as i64;
    ((signed_q10.abs() + (TAN_SCALE as i64 / 2)) / TAN_SCALE as i64) as i32
}

fn grid_pixel_color(
    sky: bool,
    distance: i32,
) -> embedded_graphics::pixelcolor::Rgb565 {
    use embedded_graphics::pixelcolor::Rgb565;

    // Start fading much earlier than before. These RGB565 bands approximate a
    // smooth blend from the grid color toward the actual plane background:
    // sky dark-blue -> light-blue, ground light-gray -> dark-gray.
    if sky {
        if distance >= 56 {
            common::dark_blue()
        } else if distance >= 44 {
            Rgb565::new(0, 16, 16)
        } else if distance >= 36 {
            Rgb565::new(0, 19, 17)
        } else if distance >= 28 {
            Rgb565::new(0, 22, 19)
        } else if distance >= 22 {
            Rgb565::new(0, 25, 20)
        } else if distance >= 16 {
            Rgb565::new(0, 28, 21)
        } else if distance >= 11 {
            Rgb565::new(0, 31, 23)
        } else if distance >= 7 {
            Rgb565::new(0, 34, 24)
        } else if distance >= 4 {
            Rgb565::new(0, 37, 25)
        } else if distance >= 2 {
            Rgb565::new(0, 39, 26)
        } else {
            Rgb565::new(0, 40, 26)
        }
    } else if distance >= 56 {
        common::light_gray()
    } else if distance >= 44 {
        Rgb565::new(21, 41, 21)
    } else if distance >= 36 {
        Rgb565::new(20, 38, 20)
    } else if distance >= 28 {
        Rgb565::new(18, 35, 18)
    } else if distance >= 22 {
        Rgb565::new(17, 33, 17)
    } else if distance >= 16 {
        Rgb565::new(16, 31, 16)
    } else if distance >= 11 {
        Rgb565::new(15, 29, 15)
    } else if distance >= 7 {
        Rgb565::new(14, 27, 14)
    } else if distance >= 4 {
        Rgb565::new(13, 25, 13)
    } else if distance >= 2 {
        Rgb565::new(12, 24, 12)
    } else {
        Rgb565::new(11, 23, 11)
    }
}

fn display_roll_pitch_f32(imu: &ImuDisplay) -> (f32, f32) {
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
    (screen_roll, screen_pitch)
}

fn display_roll_pitch(imu: &ImuDisplay) -> (i32, i32) {
    let (roll, pitch) = display_roll_pitch_f32(imu);
    (round_degrees(roll), round_degrees(pitch))
}

/// The fused yaw convention is opposite to the physical left/right direction
/// desired by the screen instruments. Flip it only at the presentation boundary
/// so fusion math and magnetic correction keep a single internal convention.
fn display_yaw(imu: &ImuDisplay) -> i32 {
    -imu.yaw_deg
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
