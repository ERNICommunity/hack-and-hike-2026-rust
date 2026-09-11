//! IMU-view projection and presentation-space attitude math.
//!
//! The worldview consumes the fused gravity/north basis directly. Euler angles
//! exist only for the numeric header and are never used to reconstruct camera
//! orientation, so the renderer has no pole branch to infer or repair.

use embedded_gui::prelude::Rect;

use crate::app::model::ImuDisplay;

pub(super) const TAN_SCALE: i32 = 1024;
pub(super) const PERSPECTIVE_NEAR_Z: f32 = 0.45;

const PI: f32 = 3.14159265358979323846;
const RAD_TO_DEG: f32 = 180.0 / PI;

// tan(50deg) gives a 100deg horizontal FOV at any viewport width.
const HORIZONTAL_HALF_FOV_TAN: f32 = 1.1917536;
const HORIZON_AT_INFINITY_DISTANCE_PX: f32 = 64.0;

type ScreenLine = ((i32, i32), (i32, i32));

#[derive(Clone, Copy)]
pub(super) struct DisplayAttitude {
    pub(super) roll_deg: f32,
    pub(super) pitch_deg: f32,
    pub(super) yaw_deg: f32,
    basis: WorldBasis,
}

/// Complete physical orientation used by the worldview. Screen +X is camera
/// forward, +Z is screen-down, and +Y completes the right-handed sensor frame.
#[derive(Clone, Copy)]
struct WorldBasis {
    gravity_screen: [f32; 3],
    north_screen: [f32; 3],
}

#[derive(Clone, Copy)]
pub(super) struct PerspectiveCamera {
    pub(super) center_x: i32,
    pub(super) center_y: i32,
    pub(super) focal_x: f32,
    pub(super) focal_y: f32,
    // Compatibility geometry for the existing horizon rasterizer and compass
    // size normalization. These are derived from gravity, not Euler pose.
    pub(super) sin_pitch: f32,
    pub(super) cos_pitch: f32,
    pub(super) sin_roll: f32,
    pub(super) cos_roll: f32,
    pub(super) pitch_offset: i32,
    // Columns of the world->camera rotation. Computing them once per frame keeps
    // every grid/glyph point to nine multiplies + six adds and no trig.
    world_x_camera: [f32; 3],
    world_y_camera: [f32; 3],
    world_z_camera: [f32; 3],
    /// World gravity (down) in renderer camera coordinates: +X right, +Y up,
    /// +Z forward.
    pub(super) gravity_camera: [f32; 3],
    // Unit-normalized screen-space horizon equation in absolute local pixels:
    // a*x + b*y + c = 0. Its absolute value is pixel distance from the horizon.
    pub(super) horizon_a_q10: i32,
    pub(super) horizon_b_q10: i32,
    pub(super) horizon_c_q10: i32,
}

pub(super) fn display_attitude(imu: &ImuDisplay) -> DisplayAttitude {
    let gravity = screen_to_camera(imu.gravity_screen);
    let roll = atan2_approx(-gravity[0], -gravity[1]) * RAD_TO_DEG;
    let horizontal = sqrt_approx(gravity[0] * gravity[0] + gravity[1] * gravity[1]);
    let pitch = -atan2_approx(gravity[2], horizontal) * RAD_TO_DEG;
    DisplayAttitude {
        roll_deg: roll,
        pitch_deg: pitch,
        // Keep the established display-heading sign for the numeric header only.
        yaw_deg: -imu.yaw_deg,
        basis: WorldBasis {
            gravity_screen: imu.gravity_screen,
            north_screen: imu.north_screen,
        },
    }
}

pub(super) fn perspective_camera(
    attitude: DisplayAttitude,
    center_x: i32,
    center_y: i32,
) -> PerspectiveCamera {
    let gravity_screen = normalize3(attitude.basis.gravity_screen).unwrap_or([0.0, 0.0, 1.0]);
    let north_screen = horizontal_unit(attitude.basis.north_screen, gravity_screen)
        .unwrap_or_else(|| initial_horizontal_reference(gravity_screen));

    // Renderer world coordinates use +Y up and -Z north. With gravity=down and
    // north known in screen coordinates, the remaining +X world axis follows
    // directly from handedness. No Euler decomposition is involved.
    let world_x_screen = cross3(gravity_screen, north_screen);
    let world_y_screen = negate3(gravity_screen);
    let world_z_screen = negate3(north_screen);

    let world_x_camera = screen_to_camera(world_x_screen);
    let world_y_camera = screen_to_camera(world_y_screen);
    let world_z_camera = screen_to_camera(world_z_screen);
    let gravity_camera = screen_to_camera(gravity_screen);
    let (focal_x, focal_y) = perspective_focals(center_x, center_y);

    // Derive the old horizon rasterizer's slope/intercept coefficients directly
    // from gravity. The focal_x/focal_y term keeps the horizon exact under the
    // renderer's anisotropic horizontal FOV.
    let scaled_gravity_x = gravity_camera[0] * focal_y / focal_x;
    let raster_norm = sqrt_approx(
        scaled_gravity_x * scaled_gravity_x + gravity_camera[1] * gravity_camera[1],
    );
    let (sin_roll, cos_roll, pitch_offset) = if raster_norm > 0.0001 {
        (
            -scaled_gravity_x / raster_norm,
            -gravity_camera[1] / raster_norm,
            round_f32(-focal_y * gravity_camera[2] / raster_norm),
        )
    } else {
        // Looking straight down puts the horizon above the viewport (all ground);
        // looking straight up puts it below (all sky). Roll is irrelevant there.
        let offscreen = center_y.max(1) * 8;
        (
            0.0,
            1.0,
            if gravity_camera[2] >= 0.0 { -offscreen } else { offscreen },
        )
    };

    let cos_pitch = sqrt_approx(
        gravity_camera[0] * gravity_camera[0] + gravity_camera[1] * gravity_camera[1],
    );
    let sin_pitch = -gravity_camera[2];

    // A camera ray through pixel offset (dx,dy) is proportional to
    // [dx/fx, -dy/fy, 1]. The horizon is where this ray is perpendicular to
    // gravity. Normalize the resulting pixel-space line so the grid fade can use
    // |a*x+b*y+c| directly as a pixel distance.
    let a_raw = gravity_camera[0] * focal_y;
    let b_raw = -gravity_camera[1] * focal_x;
    let c_center_raw = gravity_camera[2] * focal_x * focal_y;
    let line_norm = sqrt_approx(a_raw * a_raw + b_raw * b_raw);
    let (horizon_a, horizon_b, horizon_c) = if line_norm > 0.0001 {
        let inverse = 1.0 / line_norm;
        (
            a_raw * inverse,
            b_raw * inverse,
            (c_center_raw - a_raw * center_x as f32 - b_raw * center_y as f32) * inverse,
        )
    } else {
        (0.0, 0.0, HORIZON_AT_INFINITY_DISTANCE_PX)
    };

    PerspectiveCamera {
        center_x,
        center_y,
        focal_x,
        focal_y,
        sin_pitch,
        cos_pitch,
        sin_roll,
        cos_roll,
        pitch_offset,
        world_x_camera,
        world_y_camera,
        world_z_camera,
        gravity_camera,
        horizon_a_q10: round_f32(horizon_a * TAN_SCALE as f32),
        horizon_b_q10: round_f32(horizon_b * TAN_SCALE as f32),
        horizon_c_q10: round_f32(horizon_c * TAN_SCALE as f32),
    }
}

pub(super) fn world_to_camera(point: [f32; 3], camera: PerspectiveCamera) -> [f32; 3] {
    [
        point[0] * camera.world_x_camera[0]
            + point[1] * camera.world_y_camera[0]
            + point[2] * camera.world_z_camera[0],
        point[0] * camera.world_x_camera[1]
            + point[1] * camera.world_y_camera[1]
            + point[2] * camera.world_z_camera[1],
        point[0] * camera.world_x_camera[2]
            + point[1] * camera.world_y_camera[2]
            + point[2] * camera.world_z_camera[2],
    ]
}

pub(super) fn clip_camera_near(behind: [f32; 3], front: [f32; 3]) -> [f32; 3] {
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

pub(super) fn project_camera_point(
    point: [f32; 3],
    camera: PerspectiveCamera,
) -> Option<(i32, i32)> {
    if point[2] < PERSPECTIVE_NEAR_Z {
        return None;
    }

    let inverse_z = 1.0 / point[2];
    Some((
        round_f32(camera.center_x as f32 + camera.focal_x * point[0] * inverse_z),
        round_f32(camera.center_y as f32 - camera.focal_y * point[1] * inverse_z),
    ))
}

pub(super) fn project_camera_solid_line(
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

pub(super) fn clip_line(
    mut a: (i32, i32),
    mut b: (i32, i32),
    min_x: i32,
    max_x: i32,
    min_y: i32,
    max_y: i32,
) -> Option<ScreenLine> {
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

fn perspective_focals(center_x: i32, center_y: i32) -> (f32, f32) {
    let focal_x = center_x.max(1) as f32 / HORIZONTAL_HALF_FOV_TAN;
    let focal_y = center_y.max(1) as f32;
    (focal_x, focal_y)
}

pub(super) fn round_degrees(value: f32) -> i32 {
    round_f32(value)
}

pub(super) fn round_f32(value: f32) -> i32 {
    if value >= 0.0 {
        (value + 0.5) as i32
    } else {
        (value - 0.5) as i32
    }
}

pub(super) fn abs_f32(value: f32) -> f32 {
    if value < 0.0 { -value } else { value }
}

fn screen_to_camera(value: [f32; 3]) -> [f32; 3] {
    [-value[1], -value[2], value[0]]
}

fn negate3(value: [f32; 3]) -> [f32; 3] {
    [-value[0], -value[1], -value[2]]
}

fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalize3(value: [f32; 3]) -> Option<[f32; 3]> {
    let norm_sq = dot3(value, value);
    if norm_sq < 0.000001 {
        return None;
    }
    let inverse = 1.0 / sqrt_approx(norm_sq);
    Some([value[0] * inverse, value[1] * inverse, value[2] * inverse])
}

fn horizontal_unit(value: [f32; 3], gravity: [f32; 3]) -> Option<[f32; 3]> {
    let along_gravity = dot3(value, gravity);
    normalize3([
        value[0] - gravity[0] * along_gravity,
        value[1] - gravity[1] * along_gravity,
        value[2] - gravity[2] * along_gravity,
    ])
}

fn initial_horizontal_reference(gravity: [f32; 3]) -> [f32; 3] {
    horizontal_unit([1.0, 0.0, 0.0], gravity)
        .or_else(|| horizontal_unit([0.0, 1.0, 0.0], gravity))
        .unwrap_or([0.0, 0.0, 1.0])
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
