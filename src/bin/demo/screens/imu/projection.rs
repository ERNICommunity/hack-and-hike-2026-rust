//! IMU-view projection and presentation-space attitude math.
//!
//! The worldview consumes the fused gravity/north basis directly. Euler angles
//! exist only for the numeric header and are never used to reconstruct camera
//! orientation, so the renderer has no pole branch to infer or repair.

use embedded_graphics::primitives::Rectangle;
use hack_and_hike::capabilities::imu::Attitude;
use hack_and_hike_core::imu::vec3;

/// Fixed-point scale of the horizon line coefficients (10 fractional bits).
pub(super) const TAN_SCALE: i32 = 1024;
/// Points closer to the camera than this (in world units) are clipped away:
/// dividing by a depth near zero would throw them far off screen.
pub(super) const PERSPECTIVE_NEAR_Z: f32 = 0.45;

// tan(50deg) gives a 100deg horizontal FOV at any viewport width.
/// Tangent of half the horizontal field of view.
const HORIZONTAL_HALF_FOV_TAN: f32 = 1.1917536;
/// Horizon distance used when the horizon is undefined (looking straight up
/// or down), far enough from any pixel that the grid is not faded.
const HORIZON_AT_INFINITY_DISTANCE_PX: f32 = 64.0;

/// A line on screen: two `(x, y)` end points in viewport pixels.
type ScreenLine = ((i32, i32), (i32, i32));

/// What the view needs from one IMU sample.
#[derive(Clone, Copy)]
pub(super) struct DisplayAttitude {
    /// Roll for the header, in degrees.
    pub(super) roll_deg: f32,
    /// Pitch for the header, in degrees.
    pub(super) pitch_deg: f32,
    /// Yaw for the header, in degrees, -180 to 180.
    pub(super) yaw_deg: f32,
    /// The down and north directions the 3-D view is built from.
    basis: WorldBasis,
}

/// Complete physical orientation used by the worldview. Screen +X is camera
/// forward, +Z is screen-down, and +Y completes the right-handed sensor frame.
#[derive(Clone, Copy)]
struct WorldBasis {
    /// Unit vector towards the ground.
    gravity_screen: [f32; 3],
    /// Unit vector towards magnetic north, horizontal.
    north_screen: [f32; 3],
}

/// Everything needed to project world points onto the viewport for one
/// frame, computed once per frame by [`perspective_camera`].
#[derive(Clone, Copy)]
pub(super) struct PerspectiveCamera {
    /// Viewport centre, in pixels from the viewport's top-left corner.
    pub(super) center_x: i32,
    /// Row of the viewport centre; `center_x` is its column.
    pub(super) center_y: i32,
    /// Pixels per unit of `x / z` and `y / z`: the zoom of the projection.
    pub(super) focal_x: f32,
    /// Vertical pixels per unit of `y / z`: half the viewport height, which makes
    /// the vertical field of view 90°. `focal_x` is set for a 100° horizontal one.
    pub(super) focal_y: f32,
    // Compatibility geometry for the existing horizon rasterizer and compass
    // size normalization. These are derived from gravity, not Euler pose.
    /// Sine of the camera's pitch, derived from gravity. The compass uses it with
    /// `cos_pitch` to find how deep a label straight ahead lies, and sizes every
    /// label relative to that depth.
    pub(super) sin_pitch: f32,
    /// Cosine of the camera's pitch; see `sin_pitch`.
    pub(super) cos_pitch: f32,
    /// Sine of the horizon's tilt on screen. With `cos_roll` and `pitch_offset` it
    /// gives the horizon's row in every column when the ground is filled.
    pub(super) sin_roll: f32,
    /// Cosine of the horizon's tilt on screen. Its sign tells on which side of the
    /// horizon the ground is; near zero the horizon is almost vertical and the fill
    /// switches to a side-of-line test.
    pub(super) cos_roll: f32,
    /// Signed distance in pixels from the viewport centre to the horizon line: the
    /// horizon crosses the centre column at `center_y + pitch_offset / cos_roll`.
    /// Looking straight up or down it is set far off screen, so the view is all
    /// sky or all ground.
    pub(super) pitch_offset: i32,
    // Columns of the world->camera rotation. Computing them once per frame keeps
    // every grid/glyph point to nine multiplies + six adds and no trig.
    /// The world's x axis in camera coordinates: the first column of the rotation
    /// `world_to_camera` applies.
    world_x_camera: [f32; 3],
    /// The world's y axis (up, against gravity) in camera coordinates: the second
    /// column of the rotation.
    world_y_camera: [f32; 3],
    /// The world's z axis (away from north) in camera coordinates: the third
    /// column of the rotation.
    world_z_camera: [f32; 3],
    // Unit-normalized screen-space horizon equation in absolute local pixels:
    // a*x + b*y + c = 0. Its absolute value is pixel distance from the horizon.
    /// Coefficient `a` of the horizon equation, times `TAN_SCALE`: how much the
    /// distance from the horizon changes per pixel to the right.
    pub(super) horizon_a_q10: i32,
    /// Coefficient `b` of the horizon equation, times `TAN_SCALE`: how much the
    /// distance from the horizon changes per pixel down.
    pub(super) horizon_b_q10: i32,
    /// Constant `c` of the horizon equation, times `TAN_SCALE`: the signed
    /// distance of the viewport's top-left pixel from the horizon.
    pub(super) horizon_c_q10: i32,
}

/// The header angles and 3-D basis for `attitude`.
pub(super) fn display_attitude(attitude: &Attitude) -> DisplayAttitude {
    DisplayAttitude {
        roll_deg: attitude.roll_deg,
        pitch_deg: attitude.pitch_deg,
        // The numeric header shows heading with the opposite sign convention,
        // as +/-180 degrees.
        yaw_deg: wrap_degrees(-attitude.heading_deg),
        basis: WorldBasis {
            gravity_screen: attitude.down,
            north_screen: attitude.north,
        },
    }
}

/// Wrap an angle into `-180.0..=180.0`.
fn wrap_degrees(degrees: f32) -> f32 {
    let wrapped = degrees % 360.0;
    if wrapped > 180.0 {
        wrapped - 360.0
    } else if wrapped < -180.0 {
        wrapped + 360.0
    } else {
        wrapped
    }
}

/// Build the camera for a viewport centred at `(center_x, center_y)`.
pub(super) fn perspective_camera(
    attitude: DisplayAttitude,
    center_x: i32,
    center_y: i32,
) -> PerspectiveCamera {
    let gravity_screen = vec3::normalize(attitude.basis.gravity_screen).unwrap_or([0.0, 0.0, 1.0]);
    let north_screen = horizontal_unit(attitude.basis.north_screen, gravity_screen)
        .unwrap_or_else(|| initial_horizontal_reference(gravity_screen));

    // Renderer world coordinates use +Y up and -Z north. With gravity=down and
    // north known in screen coordinates, the remaining +X world axis follows
    // directly from handedness. No Euler decomposition is involved.
    let world_x_screen = vec3::cross(gravity_screen, north_screen);
    let world_y_screen = vec3::scale(gravity_screen, -1.0);
    let world_z_screen = vec3::scale(north_screen, -1.0);

    let world_x_camera = screen_to_camera(world_x_screen);
    let world_y_camera = screen_to_camera(world_y_screen);
    let world_z_camera = screen_to_camera(world_z_screen);
    let gravity_camera = screen_to_camera(gravity_screen);
    let (focal_x, focal_y) = perspective_focals(center_x, center_y);

    // Derive the old horizon rasterizer's slope/intercept coefficients directly
    // from gravity. The focal_x/focal_y term keeps the horizon exact under the
    // renderer's anisotropic horizontal FOV.
    let scaled_gravity_x = gravity_camera[0] * focal_y / focal_x;
    let raster_norm =
        libm::sqrtf(scaled_gravity_x * scaled_gravity_x + gravity_camera[1] * gravity_camera[1]);
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
            if gravity_camera[2] >= 0.0 {
                -offscreen
            } else {
                offscreen
            },
        )
    };

    let cos_pitch =
        libm::sqrtf(gravity_camera[0] * gravity_camera[0] + gravity_camera[1] * gravity_camera[1]);
    let sin_pitch = -gravity_camera[2];

    // A camera ray through pixel offset (dx,dy) is proportional to
    // [dx/fx, -dy/fy, 1]. The horizon is where this ray is perpendicular to
    // gravity. Normalize the resulting pixel-space line so the grid fade can use
    // |a*x+b*y+c| directly as a pixel distance.
    let a_raw = gravity_camera[0] * focal_y;
    let b_raw = -gravity_camera[1] * focal_x;
    let c_center_raw = gravity_camera[2] * focal_x * focal_y;
    let line_norm = libm::sqrtf(a_raw * a_raw + b_raw * b_raw);
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
        horizon_a_q10: round_f32(horizon_a * TAN_SCALE as f32),
        horizon_b_q10: round_f32(horizon_b * TAN_SCALE as f32),
        horizon_c_q10: round_f32(horizon_c * TAN_SCALE as f32),
    }
}

/// Rotate a world point (y up, -z north) into camera coordinates, where z is
/// the viewing direction and x, y map to the viewport's columns and rows.
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

/// The point where the segment from `behind` (too close) to `front` crosses
/// the near plane.
pub(super) fn clip_camera_near(behind: [f32; 3], front: [f32; 3]) -> [f32; 3] {
    let denominator = front[2] - behind[2];
    if denominator.abs() < 0.0001 {
        return [behind[0], behind[1], PERSPECTIVE_NEAR_Z];
    }
    let t = (PERSPECTIVE_NEAR_Z - behind[2]) / denominator;
    [
        behind[0] + (front[0] - behind[0]) * t,
        behind[1] + (front[1] - behind[1]) * t,
        PERSPECTIVE_NEAR_Z,
    ]
}

/// The viewport pixel of a camera-space point; `None` behind the near plane.
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

/// Project a camera-space segment drawn `width` pixels thick, clipped to the
/// near plane and to `area` so the thick line stays inside the viewport.
pub(super) fn project_camera_solid_line(
    area: Rectangle,
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
        half,
        area.size.width as i32 - 1 - half,
        half,
        area.size.height as i32 - 1 - half,
    )
}

/// Cohen-Sutherland clipping of the segment `a`-`b` to a rectangle; `None`
/// when nothing of it is inside.
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

/// Which sides of the clip rectangle a point lies beyond: bit 0 left, 1
/// right, 2 above, 3 below.
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

/// Focal lengths for a viewport with centre `(center_x, center_y)`: a 100°
/// horizontal field of view.
fn perspective_focals(center_x: i32, center_y: i32) -> (f32, f32) {
    let focal_x = center_x.max(1) as f32 / HORIZONTAL_HALF_FOV_TAN;
    let focal_y = center_y.max(1) as f32;
    (focal_x, focal_y)
}

/// Round to the nearest pixel.
pub(super) fn round_f32(value: f32) -> i32 {
    libm::roundf(value) as i32
}

/// A screen-frame vector in camera coordinates. The camera looks along the
/// screen's x axis, out of the top edge of the board.
fn screen_to_camera(value: [f32; 3]) -> [f32; 3] {
    [-value[1], -value[2], value[0]]
}

/// Component of `value` perpendicular to `gravity`, normalized.
fn horizontal_unit(value: [f32; 3], gravity: [f32; 3]) -> Option<[f32; 3]> {
    vec3::normalize(vec3::sub(
        value,
        vec3::scale(gravity, vec3::dot(value, gravity)),
    ))
}

/// Some horizontal direction, for when north is undefined.
fn initial_horizontal_reference(gravity: [f32; 3]) -> [f32; 3] {
    horizontal_unit([1.0, 0.0, 0.0], gravity)
        .or_else(|| horizontal_unit([0.0, 1.0, 0.0], gravity))
        .unwrap_or([0.0, 0.0, 1.0])
}
