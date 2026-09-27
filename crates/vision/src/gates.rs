//! Should this frame be used? Three checks on a detected face, before the
//! expensive recognizer runs on it.
//!
//! - [`framing`]: the face must fill the frame height, as the application
//!   asks of the user, and sit inside the frame.
//! - [`pose`]: the face must look at the camera: little roll (head tilt),
//!   yaw (turned left or right) or pitch (up or down), judged from the five
//!   landmarks alone.
//! - Sharpness: `quality::laplacian_variance` of the aligned crop, against
//!   a threshold the application sets.
//!
//! The limits are plain numbers in [`Limits`]; the defaults are starting
//! points to tune on the device.

use libm::atan2f;

use crate::detect::Face;

/// What is wrong with the framing, if anything.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Framing {
    /// The face fills the height and is inside the frame.
    Good,
    /// The face is too small: the user should come closer.
    TooFar,
    /// The face is taller than the frame: the user should move back.
    TooClose,
    /// The face is large enough but crosses an edge of the frame.
    OffCentre,
}

/// The pose of a face from its five landmarks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    /// The tilt of the line between the eyes, in degrees; 0 is level,
    /// positive when the image-right eye is lower.
    pub roll_degrees: f32,
    /// How far the nose tip is from the middle of the eyes, sideways, as a
    /// fraction of the distance between the eyes; 0 is frontal, positive
    /// when the nose is towards the image's right.
    pub yaw: f32,
    /// Where the nose tip lies between the eye line (0) and the mouth line
    /// (1); about 0.5 to 0.6 for a face looking at the camera, smaller
    /// when looking up, larger when looking down.
    pub pitch: f32,
}

/// The limits of the three gates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Limits {
    /// The face box must be at least this fraction of the frame height.
    pub min_height_fraction: f32,
    /// The face box may be at most this fraction of the frame height.
    pub max_height_fraction: f32,
    /// The box may reach this fraction of the frame size past an edge.
    pub edge_margin: f32,
    /// The largest roll, in degrees, either way.
    pub max_roll_degrees: f32,
    /// The largest yaw, either way.
    pub max_yaw: f32,
    /// The smallest and largest pitch.
    pub pitch_range: [f32; 2],
    /// The smallest sharpness (`quality::laplacian_variance` of the aligned
    /// 112x112 crop).
    pub min_sharpness: f32,
}

impl Limits {
    /// Starting values: the face fills 75 to 105 percent of the height,
    /// may cross an edge by 5 percent, and turns by at most 15 degrees.
    pub const DEFAULT: Self = Self {
        min_height_fraction: 0.75,
        max_height_fraction: 1.05,
        edge_margin: 0.05,
        max_roll_degrees: 15.0,
        max_yaw: 0.25,
        pitch_range: [0.35, 0.75],
        min_sharpness: 100.0,
    };
}

impl Default for Limits {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Judge the framing of `face` in a frame of `width` x `height`, all in
/// the same pixel units.
pub fn framing(face: &Face, width: f32, height: f32, limits: &Limits) -> Framing {
    if face.height < limits.min_height_fraction * height {
        return Framing::TooFar;
    }
    if face.height > limits.max_height_fraction * height {
        return Framing::TooClose;
    }
    let (margin_x, margin_y) = (limits.edge_margin * width, limits.edge_margin * height);
    let inside = face.x >= -margin_x
        && face.y >= -margin_y
        && face.x + face.width <= width + margin_x
        && face.y + face.height <= height + margin_y;
    if inside {
        Framing::Good
    } else {
        Framing::OffCentre
    }
}

/// The pose of a face from its landmarks (see [`Face::landmarks`] for the
/// order).
pub fn pose(landmarks: &[[f32; 2]; 5]) -> Pose {
    let [left_eye, right_eye, nose, left_mouth, right_mouth] = *landmarks;
    let eye_dx = right_eye[0] - left_eye[0];
    let eye_dy = right_eye[1] - left_eye[1];
    let eye_distance = libm::sqrtf(eye_dx * eye_dx + eye_dy * eye_dy).max(1e-3);
    let eyes_mid = [
        (left_eye[0] + right_eye[0]) / 2.0,
        (left_eye[1] + right_eye[1]) / 2.0,
    ];
    let mouth_mid = [
        (left_mouth[0] + right_mouth[0]) / 2.0,
        (left_mouth[1] + right_mouth[1]) / 2.0,
    ];
    let face_height = (mouth_mid[1] - eyes_mid[1]).max(1e-3);
    Pose {
        roll_degrees: atan2f(eye_dy, eye_dx).to_degrees(),
        yaw: (nose[0] - eyes_mid[0]) / eye_distance,
        pitch: (nose[1] - eyes_mid[1]) / face_height,
    }
}

impl Pose {
    /// Whether the face looks at the camera closely enough.
    pub fn is_frontal(&self, limits: &Limits) -> bool {
        self.roll_degrees.abs() <= limits.max_roll_degrees
            && self.yaw.abs() <= limits.max_yaw
            && self.pitch >= limits.pitch_range[0]
            && self.pitch <= limits.pitch_range[1]
    }
}
