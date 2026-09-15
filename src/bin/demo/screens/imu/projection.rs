//! The math of the IMU view: from the board's attitude to a perspective
//! camera, and line clipping.
//!
//! The 3-D view uses two vectors of the fused sample directly: the direction
//! of gravity (down) and the direction of north. Roll, pitch and yaw (Euler
//! angles) are only shown as numbers in the header. The view never computes
//! the camera orientation from them. So the orientation stays correct when
//! the camera looks straight up or down, where Euler angles are not unique.
//!
//! A perspective projection maps a point `(x, y, z)` in camera coordinates to
//! the pixel `center + focal * (x / z, -y / z)`. `z` is the depth: the
//! distance along the viewing direction. So far points come closer to the
//! centre and look smaller.
//!
//! Some values are fixed-point numbers in Q10 format: an `i32` that holds the
//! value times 1024 (10 bits after the binary point). Their names end in
//! `_q10`.

use embedded_graphics::primitives::Rectangle;
use hack_and_hike::capabilities::imu::Attitude;
use hack_and_hike_core::imu::vec3;

/// Fixed-point scale of the horizon line coefficients: Q10, 10 bits after the
/// binary point, so 1.0 is 1024.
pub(super) const TAN_SCALE: i32 = 1024;
/// The depth of the near plane, in world units. Points closer to the camera
/// than this plane are clipped away. A division by a depth near zero would
/// place them far outside the screen.
pub(super) const PERSPECTIVE_NEAR_Z: f32 = 0.45;

/// How much of the world the view shows from left to right, in degrees.
///
/// A flat perspective projection places a point at `tan(angle)` from the
/// centre. So near the edges of a wide view, everything moves and stretches
/// faster than in the middle, as with a wide-angle lens. At the edge, it is
/// about 2.4 times faster at 100°, 1.5 times at 70°, and 1.2 times at 50°. A
/// narrower view moves more evenly, but shows fewer compass labels at once
/// (they are 45° apart).
const HORIZONTAL_FOV_DEG: f32 = 70.0;
/// Distance from the horizon, in pixels, that every pixel gets when there is
/// no horizon line (looking straight up or down). It is larger than the fade
/// range, so the grid is not faded.
const HORIZON_AT_INFINITY_DISTANCE_PX: f32 = 64.0;
/// Limit for the distance of the viewport's top-left pixel from the horizon,
/// in pixels. Beyond this limit, every pixel is at the far end of the fade
/// anyway. Times `TAN_SCALE`, the limit still fits an `i32`, with room for
/// the per-pixel steps.
const HORIZON_C_LIMIT_PX: f32 = 1_000_000.0;

/// A line on the screen: two `(x, y)` end points in viewport pixels.
type ScreenLine = ((i32, i32), (i32, i32));

/// What the view needs from one IMU sample.
#[derive(Clone, Copy)]
pub(super) struct DisplayAttitude {
    /// Roll for the header, in degrees, -180 to 180.
    pub(super) roll_deg: f32,
    /// Pitch for the header, in degrees, -90 to 90.
    pub(super) pitch_deg: f32,
    /// Yaw for the header, in degrees, -180 to 180.
    pub(super) yaw_deg: f32,
    /// The down and north directions the 3-D view is built from.
    basis: WorldBasis,
}

/// The complete orientation of the board, used by the 3-D view. Both vectors
/// are in the screen frame of [`Attitude`]: `x` points out of the top edge
/// (the camera direction), `y` points right across the screen, and `z` points
/// into the screen. The frame is right-handed.
#[derive(Clone, Copy)]
struct WorldBasis {
    /// Unit vector towards the ground.
    gravity_screen: [f32; 3],
    /// Unit vector towards magnetic north. It is horizontal: perpendicular to
    /// `gravity_screen`.
    north_screen: [f32; 3],
}

/// Everything that is needed to project world points onto the viewport.
/// [`perspective_camera`] computes it once for each drawn frame.
#[derive(Clone, Copy)]
pub(super) struct PerspectiveCamera {
    /// Column of the viewport centre, in pixels from the viewport's left
    /// edge.
    pub(super) center_x: i32,
    /// Row of the viewport centre, in pixels from the viewport's top edge.
    pub(super) center_y: i32,
    /// Horizontal focal length: pixels per unit of `x / z`. It sets the
    /// horizontal zoom of the projection and follows from
    /// `HORIZONTAL_FOV_DEG`.
    pub(super) focal_x: f32,
    /// Vertical focal length: pixels per unit of `y / z`. It is equal to
    /// `focal_x`, so that squares stay square.
    pub(super) focal_y: f32,
    // The next five values help the ground fill in `horizon` and the label
    // sizes in `compass`. They come from gravity, not from Euler angles.
    /// Sine of the camera's pitch, from gravity. The compass uses it with
    /// `cos_pitch` to find the depth of a label straight ahead. It sizes every
    /// label relative to that depth.
    pub(super) sin_pitch: f32,
    /// Cosine of the camera's pitch; see `sin_pitch`.
    pub(super) cos_pitch: f32,
    /// Sine of the horizon's tilt on the screen. With `cos_roll` and
    /// `pitch_offset`, it gives the horizon's row in every column when the
    /// ground is filled.
    pub(super) sin_roll: f32,
    /// Cosine of the horizon's tilt on the screen. Its sign tells on which side
    /// of the horizon the ground is. Near zero, the horizon is almost vertical,
    /// and the fill uses a test for the side of the line instead.
    pub(super) cos_roll: f32,
    /// Signed distance in pixels from the viewport centre to the horizon line.
    /// The horizon crosses the centre column at
    /// `center_y + pitch_offset / cos_roll`. When the camera looks straight up
    /// or down, the value puts the horizon far outside the screen, so the view
    /// is all sky or all ground.
    pub(super) pitch_offset: i32,
    // The columns of the rotation from world to camera coordinates. They are
    // computed once per frame. Then each point of the grid or of a letter costs
    // nine multiplications and six additions, and no sine or cosine.
    /// The world's x axis in camera coordinates: the first column of the
    /// rotation that `world_to_camera` applies.
    world_x_camera: [f32; 3],
    /// The world's y axis (up, against gravity) in camera coordinates: the
    /// second column of the rotation.
    world_y_camera: [f32; 3],
    /// The world's z axis (away from north) in camera coordinates: the third
    /// column of the rotation.
    world_z_camera: [f32; 3],
    // The horizon as a line equation in viewport pixels, where `(0, 0)` is the
    // top-left pixel: a*x + b*y + c = 0. The equation is normalized
    // (a*a + b*b = 1), so |a*x + b*y + c| is the distance of pixel (x, y) from
    // the horizon, in pixels.
    /// Coefficient `a` of the horizon equation, in Q10: how much the distance
    /// from the horizon changes for each pixel to the right.
    pub(super) horizon_a_q10: i32,
    /// Coefficient `b` of the horizon equation, in Q10: how much the distance
    /// from the horizon changes for each pixel down.
    pub(super) horizon_b_q10: i32,
    /// Constant `c` of the horizon equation, in Q10: the signed distance of the
    /// viewport's top-left pixel from the horizon. It is clamped to
    /// ±`HORIZON_C_LIMIT_PX` pixels.
    pub(super) horizon_c_q10: i32,
}

/// The header angles and the two direction vectors for `attitude`.
pub(super) fn display_attitude(attitude: &Attitude) -> DisplayAttitude {
    DisplayAttitude {
        roll_deg: attitude.roll_deg,
        pitch_deg: attitude.pitch_deg,
        // The header shows the heading with the opposite sign, from -180 to
        // 180 degrees.
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

/// Build the camera for a viewport with its centre at `(center_x, center_y)`,
/// in pixels from the viewport's top-left corner. The focal lengths follow
/// from `center_x` and `HORIZONTAL_FOV_DEG`.
pub(super) fn perspective_camera(
    attitude: DisplayAttitude,
    center_x: i32,
    center_y: i32,
) -> PerspectiveCamera {
    let gravity_screen = vec3::normalize(attitude.basis.gravity_screen).unwrap_or([0.0, 0.0, 1.0]);
    let north_screen = horizontal_unit(attitude.basis.north_screen, gravity_screen)
        .unwrap_or_else(|| initial_horizontal_reference(gravity_screen));

    // The world coordinates of the view use +Y for up and -Z for north. Down
    // and north are known in screen coordinates. The cross product gives the
    // third world axis, +X. No Euler angles are needed.
    let world_x_screen = vec3::cross(gravity_screen, north_screen);
    let world_y_screen = vec3::scale(gravity_screen, -1.0);
    let world_z_screen = vec3::scale(north_screen, -1.0);

    let world_x_camera = screen_to_camera(world_x_screen);
    let world_y_camera = screen_to_camera(world_y_screen);
    let world_z_camera = screen_to_camera(world_z_screen);
    let gravity_camera = screen_to_camera(gravity_screen);
    let (focal_x, focal_y) = perspective_focals(center_x);

    // The values for the ground fill (`sin_roll`, `cos_roll`, `pitch_offset`),
    // directly from gravity. The factor focal_y / focal_x is 1 today, because
    // both focal lengths are equal. It keeps the horizon correct if they ever
    // differ.
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
        // Looking straight down puts the horizon above the viewport (all
        // ground). Looking straight up puts it below (all sky). Roll does not
        // matter there.
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

    // The camera ray through the pixel at offset (dx, dy) from the centre has
    // the direction [dx/fx, -dy/fy, 1]. The horizon is where this ray is
    // perpendicular to gravity. That gives a line in pixel coordinates. The
    // line is normalized, so the grid fade can use |a*x + b*y + c| directly
    // as a distance in pixels.
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
        // Looking almost straight up or down puts the horizon line far outside
        // the screen. There, every distance gets the same fade colour. The
        // clamp keeps the Q10 sums in `horizon::draw_line_pixels` from
        // overflowing.
        horizon_c_q10: round_f32(
            horizon_c.clamp(-HORIZON_C_LIMIT_PX, HORIZON_C_LIMIT_PX) * TAN_SCALE as f32,
        ),
    }
}

/// Rotate a world point (y up, -z north) into camera coordinates. In camera
/// coordinates, z is the viewing direction (the depth), x points to the right
/// of the viewport and y points up.
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

/// The point where the segment from `behind` (closer than the near plane) to
/// `front` crosses the near plane. The near plane is the plane at depth
/// `PERSPECTIVE_NEAR_Z`.
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

/// The viewport pixel of a point in camera coordinates. `None` when the point
/// is closer than the near plane.
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

/// Project a segment in camera coordinates that is drawn `width` pixels
/// thick. Clip it to the near plane, and to `area` minus half the width, so
/// that the thick line stays inside the viewport. `None` when nothing of the
/// line is left.
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

/// Clip the segment from `a` to `b` to the rectangle from `(min_x, min_y)` to
/// `(max_x, max_y)`, edges included. `None` when nothing of it is inside.
///
/// This is the Cohen-Sutherland algorithm. It moves an end point that is
/// outside onto an edge of the rectangle, and repeats until both end points
/// are inside, or both are outside on the same side.
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

/// Focal lengths for a viewport whose centre column is `center_x`, for the
/// horizontal field of view `HORIZONTAL_FOV_DEG`. Both are equal, so the
/// projection does not stretch one axis. The vertical field of view follows
/// from the viewport's height.
fn perspective_focals(center_x: i32) -> (f32, f32) {
    let half_fov = (HORIZONTAL_FOV_DEG / 2.0).to_radians();
    let focal = center_x.max(1) as f32 / libm::tanf(half_fov);
    (focal, focal)
}

/// Round to the nearest whole number. Values outside the `i32` range give
/// the nearest `i32` limit.
pub(super) fn round_f32(value: f32) -> i32 {
    libm::roundf(value) as i32
}

/// A vector of the screen frame in camera coordinates. The camera looks along
/// the screen's x axis, out of the top edge of the board.
fn screen_to_camera(value: [f32; 3]) -> [f32; 3] {
    [-value[1], -value[2], value[0]]
}

/// The part of `value` that is perpendicular to `gravity`, as a unit vector.
/// `gravity` must be a unit vector. `None` when `value` is (almost) parallel
/// to `gravity`.
fn horizontal_unit(value: [f32; 3], gravity: [f32; 3]) -> Option<[f32; 3]> {
    vec3::normalize(vec3::sub(
        value,
        vec3::scale(gravity, vec3::dot(value, gravity)),
    ))
}

/// Any horizontal direction. It is used when north is not defined.
fn initial_horizontal_reference(gravity: [f32; 3]) -> [f32; 3] {
    horizontal_unit([1.0, 0.0, 0.0], gravity)
        .or_else(|| horizontal_unit([0.0, 1.0, 0.0], gravity))
        .unwrap_or([0.0, 0.0, 1.0])
}
