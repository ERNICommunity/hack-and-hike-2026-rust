//! IMU-view projection and presentation-space attitude math.
//!
//! These helpers are deliberately local to the IMU screen. They are rendering
//! mechanics, not a generic graphics or sensor abstraction.

use embedded_gui::prelude::Rect;

use crate::app::model::ImuDisplay;

pub(super) const TAN_SCALE: i32 = 1024;
const TAN_STEP_DEG: i32 = 5;
const TAN_MAX_DEG: i32 = 80;
const TAN_Q10: [i32; 17] = [
    0, 90, 181, 274, 373, 477, 591, 717, 859, 1024, 1220, 1462, 1774, 2196, 2814,
    3822, 5807,
];

const PI: f32 = 3.14159265358979323846;
const RAD_TO_DEG: f32 = 180.0 / PI;
const DEG_TO_RAD: f32 = PI / 180.0;
pub(super) const PERSPECTIVE_NEAR_Z: f32 = 0.45;

// Keep the preferred rectilinear renderer, but narrow only the horizontal field
// of view to reduce its unavoidable sec(theta)^2 yaw-speed increase toward the
// edges. tan(50deg) gives a 100deg horizontal FOV at any viewport width. Roll is
// applied after projection, preserving the preferred horizon angle exactly.
const HORIZONTAL_HALF_FOV_TAN: f32 = 1.1917536;

type ScreenLine = ((i32, i32), (i32, i32));

#[derive(Clone, Copy)]
pub(super) struct DisplayAttitude {
    pub(super) roll_deg: f32,
    pub(super) pitch_deg: f32,
    pub(super) yaw_deg: i32,
}

#[derive(Clone, Copy)]
pub(super) struct PerspectiveCamera {
    pub(super) center_x: i32,
    pub(super) center_y: i32,
    pub(super) focal_x: f32,
    pub(super) focal_y: f32,
    pub(super) sin_yaw: f32,
    pub(super) cos_yaw: f32,
    pub(super) sin_pitch: f32,
    pub(super) cos_pitch: f32,
    pub(super) sin_roll: f32,
    pub(super) cos_roll: f32,
    pub(super) pitch_offset: i32,
    // Q10 coefficients for the displayed horizon's implicit line:
    // a*x + b*y + c = 0. Perpendicular screen distance from this line is a
    // cheap proxy for inverse world depth on the sky/ground planes.
    pub(super) horizon_a_q10: i32,
    pub(super) horizon_b_q10: i32,
    pub(super) horizon_c_q10: i32,
}

pub(super) fn display_attitude(imu: &ImuDisplay) -> DisplayAttitude {
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

pub(super) fn perspective_camera(
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

pub(super) fn world_to_camera(point: [f32; 3], camera: PerspectiveCamera) -> [f32; 3] {
    let yaw_x = camera.cos_yaw * point[0] - camera.sin_yaw * point[2];
    let yaw_z = camera.sin_yaw * point[0] + camera.cos_yaw * point[2];
    let pitched_y = camera.cos_pitch * point[1] - camera.sin_pitch * yaw_z;
    let pitched_z = camera.sin_pitch * point[1] + camera.cos_pitch * yaw_z;

    // Keep roll out of 3D camera space. Applying it after the anisotropic
    // perspective projection preserves the exact roll angle of the preferred
    // renderer while still allowing a narrower horizontal FOV.
    [yaw_x, pitched_y, pitched_z]
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
