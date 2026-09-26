//! Face verification: does the face in the camera image look like the face
//! that was enrolled (recorded) before?
//!
//! The method is a classic one without a neural network: histograms of
//! local binary patterns (LBP), compared with the chi-square distance.
//!
//! 1. A [`Patch`] is a small grey image of the square in the middle of the
//!    camera frame. The user holds their face in this square; there is no
//!    face detection. Each patch pixel is the mean brightness of a 2x2
//!    block of camera pixels, which also reduces the sensor noise.
//! 2. [`Codes`] gives each patch pixel an 8-bit code: one bit for each of
//!    its 8 neighbours, set when the neighbour is brighter than the pixel.
//!    Brighter or darker light changes every pixel by about the same amount,
//!    so the codes stay the same. That is why LBP works under different
//!    light.
//! 3. [`Features`] divides a window of the codes into a grid of cells and
//!    counts the codes in each cell: one histogram per cell. The histograms
//!    describe the texture of each part of the face: eyes, nose, mouth.
//! 4. [`Features::distance`] compares two faces: 0 means the histograms are
//!    the same, 2 means they have nothing in common.
//!
//! A face is never in exactly the same place twice. So [`best_distance`]
//! moves the window over a few [`OFFSETS`] and keeps the smallest distance.
//! [`FaceLock`] decides when to keep an enrolment sample, when to unlock
//! and when to lock again.
//!
//! The firmware must empty the camera's buffer every few milliseconds. So
//! the slow steps work one row or one comparison at a time, and
//! [`best_distance`] calls back between comparisons.

/// Side of the [`Patch`] in pixels.
pub const PATCH_SIZE: usize = 96;
/// Camera pixels per patch pixel along each axis. Each patch pixel is the
/// mean of a `SCALE` x `SCALE` block.
pub const SCALE: usize = 2;
/// Side of the square of camera pixels that a patch covers: 192.
pub const SOURCE_SIZE: usize = PATCH_SIZE * SCALE;
/// Side of the [`Codes`] image. The pixels on the patch border have no
/// neighbour on one side, so they get no code: 94.
pub const CODES_SIZE: usize = PATCH_SIZE - 2;
/// Side of the window of codes that [`Features`] describe: 80.
pub const WINDOW: usize = 80;
/// Cells along each side of the window.
pub const GRID: usize = 5;
/// Side of one cell, in codes: 16.
pub const CELL: usize = WINDOW / GRID;
/// Cells in the window: 25.
pub const CELLS: usize = GRID * GRID;
/// Histogram bins per cell: 58 uniform patterns and one bin for all other
/// patterns (see [`UNIFORM_BIN`]).
pub const BINS: usize = 59;
/// The largest window offset inside the codes: 14.
pub const MAX_OFFSET: usize = CODES_SIZE - WINDOW;
/// The offset of the window in the middle of the codes: 7. Enrolment uses
/// it, and the guide box on the screen shows it.
pub const CENTER_OFFSET: usize = MAX_OFFSET / 2;
/// The window offsets that [`best_distance`] tries, along each axis. Nine
/// windows in all. One patch pixel is two camera pixels, so the face may be
/// up to 14 camera pixels away from the middle.
pub const OFFSETS: [usize; 3] = [0, CENTER_OFFSET, MAX_OFFSET];
/// How much brighter than the centre pixel a neighbour must be for its bit
/// to be set, in grey levels. Without it, sensor noise in flat areas such
/// as the cheeks would set random bits.
pub const NOISE_TOLERANCE: u8 = 3;

/// Codes counted in one [`Features`]: 6,400.
const SAMPLES: u32 = (CELLS * CELL * CELL) as u32;

const _: () = assert!(WINDOW.is_multiple_of(GRID));
const _: () = assert!(WINDOW <= CODES_SIZE);

/// The brightness (luma) of one big-endian RGB565 pixel, 0 to 255.
///
/// The weights are the usual ones for video (ITU-R BT.601): green counts
/// most, blue least.
pub fn luma(high: u8, low: u8) -> u8 {
    let pixel = u16::from_be_bytes([high, low]);
    let red = u32::from(pixel >> 11);
    let green = u32::from((pixel >> 5) & 0x3F);
    let blue = u32::from(pixel & 0x1F);
    // Stretch the 5- and 6-bit values to 8 bits, so white becomes 255.
    let red = (red << 3) | (red >> 2);
    let green = (green << 2) | (green >> 4);
    let blue = (blue << 3) | (blue >> 2);
    // The weights add up to 256, so the result is at most 255.
    ((77 * red + 150 * green + 29 * blue) >> 8) as u8
}

/// A grey image of the square in the middle of the camera frame, at half
/// the camera's resolution.
#[derive(Clone)]
pub struct Patch {
    /// Brightness of each pixel, row by row.
    pixels: [u8; PATCH_SIZE * PATCH_SIZE],
}

/// Whether a [`Patch`] is good enough to compare faces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quality {
    /// Bright enough, not too bright, with enough contrast.
    Good,
    /// Too dark: the room is dark, or the camera is covered.
    TooDark,
    /// Too bright: a lamp or a window shines into the camera.
    TooBright,
    /// Too little contrast: a wall, or a face much too close.
    Flat,
}

/// Below this mean brightness, a patch is [`Quality::TooDark`].
const MIN_MEAN: u32 = 35;
/// Above this mean brightness, a patch is [`Quality::TooBright`].
const MAX_MEAN: u32 = 225;
/// Below this standard deviation of the brightness, a patch is
/// [`Quality::Flat`].
const MIN_STD_DEV: f32 = 10.0;

impl Patch {
    /// A black patch.
    pub const fn new() -> Self {
        Self {
            pixels: [0; PATCH_SIZE * PATCH_SIZE],
        }
    }

    /// Fill patch row `row` from two camera rows.
    ///
    /// `upper` and `lower` are the big-endian RGB565 bytes of camera rows
    /// `SCALE * row` and `SCALE * row + 1` of the square, each
    /// [`SOURCE_SIZE`] pixels long.
    ///
    /// # Panics
    ///
    /// When `row` is [`PATCH_SIZE`] or more, or a slice has the wrong
    /// length.
    pub fn fill_row(&mut self, row: usize, upper: &[u8], lower: &[u8]) {
        const _: () = assert!(SCALE == 2);
        assert_eq!(upper.len(), SOURCE_SIZE * 2, "upper camera row length");
        assert_eq!(lower.len(), SOURCE_SIZE * 2, "lower camera row length");
        let out = &mut self.pixels[row * PATCH_SIZE..(row + 1) * PATCH_SIZE];
        // Four bytes are two camera pixels side by side.
        for ((pixel, top), bottom) in out
            .iter_mut()
            .zip(upper.chunks_exact(4))
            .zip(lower.chunks_exact(4))
        {
            let sum = u16::from(luma(top[0], top[1]))
                + u16::from(luma(top[2], top[3]))
                + u16::from(luma(bottom[0], bottom[1]))
                + u16::from(luma(bottom[2], bottom[3]));
            // Round to the nearest value.
            *pixel = ((sum + 2) / 4) as u8;
        }
    }

    /// The brightness of the pixel in column `x` of row `y`.
    fn at(&self, x: usize, y: usize) -> u8 {
        self.pixels[y * PATCH_SIZE + x]
    }

    /// Check the brightness and contrast of the middle window: the part of
    /// the patch that enrolment uses and the guide box shows.
    pub fn quality(&self) -> Quality {
        // Code (x, y) belongs to patch pixel (x + 1, y + 1).
        let start = CENTER_OFFSET + 1;
        let mut sum = 0_u64;
        let mut sum_of_squares = 0_u64;
        for y in start..start + WINDOW {
            for x in start..start + WINDOW {
                let value = u64::from(self.at(x, y));
                sum += value;
                sum_of_squares += value * value;
            }
        }
        let count = (WINDOW * WINDOW) as u64;
        let mean = sum / count;
        // Variance = mean of the squares - square of the mean.
        let exact_mean = sum as f32 / count as f32;
        let variance = sum_of_squares as f32 / count as f32 - exact_mean * exact_mean;
        let std_dev = libm::sqrtf(variance.max(0.0));
        if mean < u64::from(MIN_MEAN) {
            Quality::TooDark
        } else if mean > u64::from(MAX_MEAN) {
            Quality::TooBright
        } else if std_dev < MIN_STD_DEV {
            Quality::Flat
        } else {
            Quality::Good
        }
    }
}

impl Default for Patch {
    fn default() -> Self {
        Self::new()
    }
}

/// Number of bit changes when an 8-bit pattern is read around the circle,
/// from bit 7 back to bit 0.
const fn transitions(code: u8) -> u32 {
    (code ^ code.rotate_left(1)).count_ones()
}

/// The histogram bin of each 8-bit code.
///
/// A code is "uniform" when it has at most two bit changes around the
/// circle, for example `00111000`. Uniform codes stand for edges, corners,
/// spots and flat areas: most codes in a face are uniform. Each of the 58
/// uniform codes gets its own bin, 0 to 57, in increasing order. All other
/// codes share bin 58. This makes the histograms smaller and less noisy.
pub const UNIFORM_BIN: [u8; 256] = {
    let mut table = [(BINS - 1) as u8; 256];
    let mut next = 0;
    let mut code = 0;
    while code < 256 {
        if transitions(code as u8) <= 2 {
            table[code] = next;
            next += 1;
        }
        code += 1;
    }
    assert!(next as usize == BINS - 1);
    table
};

/// The local binary pattern code of every inner pixel of a [`Patch`],
/// already mapped to its histogram bin with [`UNIFORM_BIN`].
#[derive(Clone)]
pub struct Codes {
    /// The bin of each code, row by row. Code (x, y) belongs to patch pixel
    /// (x + 1, y + 1).
    bins: [u8; CODES_SIZE * CODES_SIZE],
}

impl Codes {
    /// Codes for a black patch: all flat.
    pub const fn new() -> Self {
        Self {
            bins: [0; CODES_SIZE * CODES_SIZE],
        }
    }

    /// Compute row `row` of the codes from `patch`. Call it for every row,
    /// 0 to [`CODES_SIZE`] - 1, before [`Features::compute`].
    ///
    /// # Panics
    ///
    /// When `row` is [`CODES_SIZE`] or more.
    pub fn compute_row(&mut self, patch: &Patch, row: usize) {
        assert!(row < CODES_SIZE, "code row {row} out of range");
        // The 8 neighbours, clockwise from the top-left one. The bit number
        // is the position in this list.
        const NEIGHBOURS: [(usize, usize); 8] = [
            (0, 0),
            (1, 0),
            (2, 0),
            (2, 1),
            (2, 2),
            (1, 2),
            (0, 2),
            (0, 1),
        ];
        let out = &mut self.bins[row * CODES_SIZE..(row + 1) * CODES_SIZE];
        for (x, bin) in out.iter_mut().enumerate() {
            let limit = patch.at(x + 1, row + 1).saturating_add(NOISE_TOLERANCE);
            let mut code = 0_u8;
            for (bit, (dx, dy)) in NEIGHBOURS.iter().enumerate() {
                if patch.at(x + dx, row + dy) >= limit {
                    code |= 1 << bit;
                }
            }
            *bin = UNIFORM_BIN[usize::from(code)];
        }
    }

    /// Compute all rows at once. The firmware calls
    /// [`compute_row`](Self::compute_row) instead, so it can empty the
    /// camera buffer between rows.
    pub fn compute(&mut self, patch: &Patch) {
        for row in 0..CODES_SIZE {
            self.compute_row(patch, row);
        }
    }
}

impl Default for Codes {
    fn default() -> Self {
        Self::new()
    }
}

/// The description of one face: a histogram of codes for each cell of a
/// window.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Features {
    /// The histograms, one per cell, row by row. Each holds `CELL * CELL`
    /// codes.
    histograms: [[u16; BINS]; CELLS],
}

impl Features {
    /// Features with empty histograms. Use them only as storage for
    /// [`compute`](Self::compute).
    pub const EMPTY: Self = Self {
        histograms: [[0; BINS]; CELLS],
    };

    /// Describe the window of `codes` whose top-left code is at
    /// (`offset_x`, `offset_y`).
    ///
    /// # Panics
    ///
    /// When an offset is larger than [`MAX_OFFSET`].
    pub fn compute(&mut self, codes: &Codes, offset_x: usize, offset_y: usize) {
        assert!(
            offset_x <= MAX_OFFSET && offset_y <= MAX_OFFSET,
            "window offset ({offset_x}, {offset_y}) out of range"
        );
        self.histograms = [[0; BINS]; CELLS];
        for y in 0..WINDOW {
            let start = (offset_y + y) * CODES_SIZE + offset_x;
            let row = &codes.bins[start..start + WINDOW];
            let cells = &mut self.histograms[(y / CELL) * GRID..(y / CELL + 1) * GRID];
            for (cell, bins) in cells.iter_mut().zip(row.chunks_exact(CELL)) {
                for &bin in bins {
                    cell[usize::from(bin)] += 1;
                }
            }
        }
    }

    /// The chi-square distance between two faces, from 0 (the same
    /// histograms) to 2 (no code in common).
    ///
    /// For each bin, the squared difference of the two counts is divided by
    /// their sum. So a difference in a rare code counts more than the same
    /// difference in a frequent code. The sum over all bins is divided by
    /// the number of codes, so the result does not depend on the window
    /// size.
    pub fn distance(&self, other: &Self) -> f32 {
        let mut sum = 0.0_f32;
        for (&a, &b) in self
            .histograms
            .as_flattened()
            .iter()
            .zip(other.histograms.as_flattened())
        {
            let total = u32::from(a) + u32::from(b);
            if total != 0 {
                let difference = f32::from(a) - f32::from(b);
                sum += difference * difference / total as f32;
            }
        }
        sum / SAMPLES as f32
    }
}

/// The smallest distance between the face in `codes` and any of the
/// `templates`, over all windows at [`OFFSETS`]. `None` when there are no
/// templates.
///
/// `probe` is working memory for the features of each window; it is large,
/// so the caller provides it. `between` runs after each window, while the
/// work is not finished. The firmware empties the camera buffer there.
pub fn best_distance(
    codes: &Codes,
    templates: &[Features],
    probe: &mut Features,
    mut between: impl FnMut(),
) -> Option<f32> {
    if templates.is_empty() {
        return None;
    }
    let mut best = f32::INFINITY;
    for offset_y in OFFSETS {
        for offset_x in OFFSETS {
            probe.compute(codes, offset_x, offset_y);
            for template in templates {
                best = best.min(probe.distance(template));
            }
            between();
        }
    }
    Some(best)
}

/// Enrolment samples, and so templates, of one face.
pub const TEMPLATES: usize = 5;
/// Time between two enrolment samples, in milliseconds. The user moves a
/// little in between, so the templates cover small changes of pose.
pub const SAMPLE_INTERVAL_MS: u64 = 400;
/// Matching frames in a row needed to unlock. One lucky frame is not
/// enough.
pub const MATCHES_TO_UNLOCK: u8 = 3;
/// How long the lock stays open after the last matching frame, in
/// milliseconds.
pub const UNLOCK_HOLD_MS: u64 = 5_000;
/// The smallest threshold that [`FaceLock::set_threshold`] accepts.
pub const MIN_THRESHOLD: f32 = 0.05;
/// The largest threshold that [`FaceLock::set_threshold`] accepts.
pub const MAX_THRESHOLD: f32 = 1.0;

/// Where the lock is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum State {
    /// No face is enrolled. Nothing can unlock.
    Empty,
    /// Enrolment is running.
    Enrolling {
        /// Samples kept so far, 0 to [`TEMPLATES`] - 1.
        samples: usize,
        /// When the next sample may be kept, in milliseconds.
        next_ms: u64,
    },
    /// A face is enrolled, and the lock is closed.
    Locked {
        /// Matching frames in a row so far.
        streak: u8,
    },
    /// The enrolled face was recognized.
    Unlocked {
        /// When the lock closes again, unless the face matches again
        /// before, in milliseconds.
        until_ms: u64,
    },
}

/// What one camera frame showed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Observation {
    /// No usable face: the [`Quality`] of the patch was not good.
    NoFace,
    /// A usable patch.
    Face {
        /// The [`best_distance`] to the enrolled face. `None` while nothing
        /// is enrolled or enrolment is running: then it is not computed.
        distance: Option<f32>,
    },
}

/// Something that changed in [`FaceLock::update`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    /// Keep the features of this frame's middle window as template `index`.
    SampleKept {
        /// Which template to replace, 0 to [`TEMPLATES`] - 1.
        index: usize,
        /// Whether this was the last sample. The lock is closed now.
        complete: bool,
    },
    /// The enrolled face was recognized, and the lock opened.
    Unlocked {
        /// The distance of the frame that opened it.
        distance: f32,
    },
    /// The lock closed again.
    Locked,
}

/// The rules of the face lock: enrolment, unlocking and locking again.
///
/// It does not hold the templates; the firmware keeps them in PSRAM and
/// stores a template when [`update`](Self::update) returns
/// [`Event::SampleKept`].
#[derive(Clone, Debug)]
pub struct FaceLock {
    /// Where the lock is.
    state: State,
    /// The largest distance that still counts as a match.
    threshold: f32,
}

impl FaceLock {
    /// An empty lock that matches faces up to `threshold`. The threshold is
    /// clamped to [`MIN_THRESHOLD`]..=[`MAX_THRESHOLD`].
    pub fn new(threshold: f32) -> Self {
        Self {
            state: State::Empty,
            threshold: threshold.clamp(MIN_THRESHOLD, MAX_THRESHOLD),
        }
    }

    /// Where the lock is.
    pub const fn state(&self) -> State {
        self.state
    }

    /// The largest distance that still counts as a match.
    pub const fn threshold(&self) -> f32 {
        self.threshold
    }

    /// Change the threshold, clamped to
    /// [`MIN_THRESHOLD`]..=[`MAX_THRESHOLD`].
    pub fn set_threshold(&mut self, threshold: f32) {
        self.threshold = threshold.clamp(MIN_THRESHOLD, MAX_THRESHOLD);
    }

    /// Start a new enrolment. It replaces the enrolled face, and it closes
    /// the lock.
    pub fn start_enrolling(&mut self, now_ms: u64) {
        self.state = State::Enrolling {
            samples: 0,
            next_ms: now_ms,
        };
    }

    /// Whether [`update`](Self::update) needs the distance of a usable
    /// patch. It does after enrolment.
    pub const fn wants_distance(&self) -> bool {
        matches!(self.state, State::Locked { .. } | State::Unlocked { .. })
    }

    /// Take the observation of one frame at time `now_ms`, and report what
    /// changed.
    pub fn update(&mut self, now_ms: u64, observation: Observation) -> Option<Event> {
        let face = matches!(observation, Observation::Face { .. });
        let matched = matches!(
            observation,
            Observation::Face { distance: Some(distance) } if distance <= self.threshold
        );
        let distance = match observation {
            Observation::Face {
                distance: Some(distance),
            } => distance,
            _ => f32::INFINITY,
        };
        match self.state {
            State::Empty => None,
            State::Enrolling { samples, next_ms } => {
                if !face || now_ms < next_ms {
                    return None;
                }
                let complete = samples + 1 == TEMPLATES;
                self.state = if complete {
                    State::Locked { streak: 0 }
                } else {
                    State::Enrolling {
                        samples: samples + 1,
                        next_ms: now_ms + SAMPLE_INTERVAL_MS,
                    }
                };
                Some(Event::SampleKept {
                    index: samples,
                    complete,
                })
            }
            State::Locked { streak } => {
                if !matched {
                    self.state = State::Locked { streak: 0 };
                    return None;
                }
                let streak = streak + 1;
                if streak < MATCHES_TO_UNLOCK {
                    self.state = State::Locked { streak };
                    return None;
                }
                self.state = State::Unlocked {
                    until_ms: now_ms + UNLOCK_HOLD_MS,
                };
                Some(Event::Unlocked { distance })
            }
            State::Unlocked { until_ms } => {
                if matched {
                    self.state = State::Unlocked {
                        until_ms: now_ms + UNLOCK_HOLD_MS,
                    };
                    None
                } else if now_ms >= until_ms {
                    self.state = State::Locked { streak: 0 };
                    Some(Event::Locked)
                } else {
                    None
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A patch whose pixel (x, y) is `value(x, y)`.
    fn patch(mut value: impl FnMut(usize, usize) -> u8) -> Patch {
        let mut patch = Patch::new();
        for y in 0..PATCH_SIZE {
            for x in 0..PATCH_SIZE {
                patch.pixels[y * PATCH_SIZE + x] = value(x, y);
            }
        }
        patch
    }

    /// A face-like test image: a smooth background with dark spots for eyes
    /// and mouth. `shift` moves the whole image right and down.
    fn face(x: usize, y: usize, shift: usize) -> u8 {
        let (x, y) = ((x + 100 - shift) as i32, (y + 100 - shift) as i32);
        let near = |cx: i32, cy: i32, r: i32| (x - cx).pow(2) + (y - cy).pow(2) < r * r;
        if near(135, 135, 7) || near(165, 135, 7) || near(150, 170, 9) {
            40
        } else {
            // A gentle diagonal gradient with some texture.
            (100 + (x + y) / 4 + 12 * ((x / 5 + y / 7) % 2)) as u8
        }
    }

    /// The features of the middle window of `patch`.
    fn middle_features(patch: &Patch) -> Features {
        let mut codes = Codes::new();
        codes.compute(patch);
        let mut features = Features::EMPTY;
        features.compute(&codes, CENTER_OFFSET, CENTER_OFFSET);
        features
    }

    /// The best distance between `probe` and the middle window of
    /// `enrolled`.
    fn best(enrolled: &Patch, probe: &Patch) -> f32 {
        let templates = [middle_features(enrolled)];
        let mut codes = Codes::new();
        codes.compute(probe);
        let mut scratch = Features::EMPTY;
        best_distance(&codes, &templates, &mut scratch, || {}).unwrap()
    }

    #[test]
    fn there_are_58_uniform_codes_in_increasing_order() {
        let uniform: [usize; 256] =
            core::array::from_fn(|code| usize::from(transitions(code as u8) <= 2));
        assert_eq!(uniform.iter().sum::<usize>(), 58);
        let mut expected = 0;
        for code in 0..256 {
            if uniform[code] == 1 {
                assert_eq!(usize::from(UNIFORM_BIN[code]), expected, "code {code:08b}");
                expected += 1;
            } else {
                assert_eq!(usize::from(UNIFORM_BIN[code]), BINS - 1, "code {code:08b}");
            }
        }
        assert_eq!(UNIFORM_BIN[0], 0);
        assert_eq!(usize::from(UNIFORM_BIN[0xFF]), BINS - 2);
    }

    #[test]
    fn luma_covers_the_full_range() {
        assert_eq!(luma(0x00, 0x00), 0);
        assert_eq!(luma(0xFF, 0xFF), 255);
        let red = luma(0xF8, 0x00);
        let green = luma(0x07, 0xE0);
        let blue = luma(0x00, 0x1F);
        assert!(green > red && red > blue, "{red} {green} {blue}");
    }

    #[test]
    fn a_patch_row_is_the_mean_of_two_by_two_blocks() {
        // Upper row: white, black, white, black, ...; lower row: all white.
        let mut upper = [0_u8; SOURCE_SIZE * 2];
        for pixel in upper.chunks_exact_mut(4) {
            pixel[..2].copy_from_slice(&[0xFF, 0xFF]);
        }
        let lower = [0xFF_u8; SOURCE_SIZE * 2];
        let mut patch = Patch::new();
        patch.fill_row(3, &upper, &lower);
        // (255 + 0 + 255 + 255) / 4, rounded.
        assert!((0..PATCH_SIZE).all(|x| patch.at(x, 3) == 191));
        assert!((0..PATCH_SIZE).all(|x| patch.at(x, 2) == 0));
    }

    #[test]
    fn a_face_matches_itself() {
        let face = patch(|x, y| face(x, y, 0));
        assert_eq!(
            middle_features(&face).distance(&middle_features(&face)),
            0.0
        );
        assert_eq!(best(&face, &face), 0.0);
    }

    #[test]
    fn brighter_light_does_not_change_the_features() {
        let normal = patch(|x, y| face(x, y, 0));
        let brighter = patch(|x, y| face(x, y, 0) + 30);
        assert_eq!(middle_features(&normal), middle_features(&brighter));
    }

    #[test]
    fn sensor_noise_in_flat_areas_is_ignored() {
        let flat = patch(|_, _| 120);
        let noisy = patch(|x, y| 120 + ((x * 7 + y * 13) % 3) as u8);
        assert_eq!(middle_features(&flat), middle_features(&noisy));
    }

    #[test]
    fn a_moved_face_matches_at_another_offset() {
        let enrolled = patch(|x, y| face(x, y, 0));
        let moved = patch(|x, y| face(x, y, CENTER_OFFSET));
        // In the middle window alone, the moved face looks different...
        let middle = middle_features(&enrolled).distance(&middle_features(&moved));
        assert!(middle > 0.0, "{middle}");
        // ...but the window at the matching offset finds it.
        assert_eq!(best(&enrolled, &moved), 0.0);
    }

    #[test]
    fn different_images_are_farther_than_the_same_image() {
        let vertical = patch(|x, _| if x % 6 < 3 { 60 } else { 180 });
        let horizontal = patch(|_, y| if y % 6 < 3 { 60 } else { 180 });
        let face = patch(|x, y| face(x, y, 0));
        // Two thirds of the stripe codes are flat (code 0) in both
        // images. The other third differs, so the distance is 2/3.
        let stripes = best(&vertical, &horizontal);
        assert!((stripes - 2.0 / 3.0).abs() < 0.05, "{stripes}");
        // The test face is smooth, so most of its codes are flat too.
        for other in [&vertical, &horizontal] {
            let distance = best(&face, other);
            assert!(distance > 0.2, "{distance}");
        }
    }

    #[test]
    fn distance_is_symmetric_and_at_most_two() {
        let a = middle_features(&patch(|x, y| face(x, y, 0)));
        let b = middle_features(&patch(|x, _| if x % 6 < 3 { 60 } else { 180 }));
        assert_eq!(a.distance(&b), b.distance(&a));
        // A flat image has only code 0. In a gradient that gets brighter to
        // the right by the noise tolerance (3 per pixel), every pixel has the
        // code of its three right neighbours. No code is in both.
        let flat = middle_features(&patch(|_, _| 100));
        let gradient = middle_features(&patch(|x, _| (3 * x.saturating_sub(CENTER_OFFSET)) as u8));
        assert!((flat.distance(&gradient) - 2.0).abs() < 1e-6);
    }

    #[test]
    fn best_distance_needs_templates() {
        let codes = Codes::new();
        let mut scratch = Features::EMPTY;
        assert_eq!(best_distance(&codes, &[], &mut scratch, || {}), None);
    }

    #[test]
    fn best_distance_calls_back_after_each_window() {
        let codes = Codes::new();
        let templates = [Features::EMPTY];
        let mut scratch = Features::EMPTY;
        let mut calls = 0;
        best_distance(&codes, &templates, &mut scratch, || calls += 1);
        assert_eq!(calls, OFFSETS.len() * OFFSETS.len());
    }

    #[test]
    fn quality_rejects_dark_bright_and_flat_images() {
        assert_eq!(patch(|_, _| 10).quality(), Quality::TooDark);
        assert_eq!(patch(|_, _| 250).quality(), Quality::TooBright);
        assert_eq!(patch(|_, _| 128).quality(), Quality::Flat);
        assert_eq!(patch(|x, y| face(x, y, 0)).quality(), Quality::Good);
    }

    /// A lock with a finished enrolment, at time `now`.
    fn enrolled_lock(now: u64) -> FaceLock {
        let mut lock = FaceLock::new(0.3);
        lock.start_enrolling(now);
        for i in 0..TEMPLATES as u64 {
            lock.update(
                now + i * SAMPLE_INTERVAL_MS,
                Observation::Face { distance: None },
            );
        }
        assert_eq!(lock.state(), State::Locked { streak: 0 });
        lock
    }

    /// An observation of a face at `distance`.
    fn seen(distance: f32) -> Observation {
        Observation::Face {
            distance: Some(distance),
        }
    }

    #[test]
    fn an_empty_lock_never_opens() {
        let mut lock = FaceLock::new(0.3);
        assert!(!lock.wants_distance());
        for now in 0..10 {
            assert_eq!(lock.update(now, seen(0.0)), None);
        }
        assert_eq!(lock.state(), State::Empty);
    }

    #[test]
    fn enrolment_keeps_spaced_samples_of_usable_frames() {
        let mut lock = FaceLock::new(0.3);
        lock.start_enrolling(1_000);
        let face = Observation::Face { distance: None };
        assert_eq!(lock.update(1_000, Observation::NoFace), None);
        assert_eq!(
            lock.update(1_010, face),
            Some(Event::SampleKept {
                index: 0,
                complete: false
            })
        );
        // Too soon after the first sample.
        assert_eq!(lock.update(1_010 + SAMPLE_INTERVAL_MS - 1, face), None);
        let mut now = 1_010;
        for index in 1..TEMPLATES {
            now += SAMPLE_INTERVAL_MS;
            assert_eq!(
                lock.update(now, face),
                Some(Event::SampleKept {
                    index,
                    complete: index == TEMPLATES - 1
                })
            );
        }
        assert_eq!(lock.state(), State::Locked { streak: 0 });
        assert!(lock.wants_distance());
    }

    #[test]
    fn unlocking_needs_matches_in_a_row() {
        let mut lock = enrolled_lock(0);
        let mut now = 10_000;
        for _ in 1..MATCHES_TO_UNLOCK {
            assert_eq!(lock.update(now, seen(0.2)), None);
            now += 30;
        }
        // A miss starts the count again.
        assert_eq!(lock.update(now, seen(0.5)), None);
        assert_eq!(lock.state(), State::Locked { streak: 0 });
        for _ in 1..MATCHES_TO_UNLOCK {
            now += 30;
            assert_eq!(lock.update(now, seen(0.2)), None);
        }
        now += 30;
        assert_eq!(
            lock.update(now, seen(0.25)),
            Some(Event::Unlocked { distance: 0.25 })
        );
    }

    #[test]
    fn frames_without_a_face_reset_the_streak() {
        let mut lock = enrolled_lock(0);
        lock.update(10_000, seen(0.1));
        lock.update(10_030, Observation::NoFace);
        assert_eq!(lock.state(), State::Locked { streak: 0 });
    }

    #[test]
    fn the_lock_stays_open_while_the_face_matches_and_closes_later() {
        let mut lock = enrolled_lock(0);
        let mut now = 10_000;
        for _ in 0..MATCHES_TO_UNLOCK {
            lock.update(now, seen(0.1));
            now += 30;
        }
        assert!(matches!(lock.state(), State::Unlocked { .. }));
        // Matching frames keep it open past the first hold time.
        let last_match = now + UNLOCK_HOLD_MS;
        while now <= last_match {
            assert_eq!(lock.update(now, seen(0.1)), None);
            now += 500;
        }
        // Then the face leaves.
        assert_eq!(
            lock.update(last_match + UNLOCK_HOLD_MS - 1, Observation::NoFace),
            None
        );
        assert_eq!(
            lock.update(last_match + UNLOCK_HOLD_MS, Observation::NoFace),
            Some(Event::Locked)
        );
        assert_eq!(lock.state(), State::Locked { streak: 0 });
    }

    #[test]
    fn enrolling_again_closes_the_lock() {
        let mut lock = enrolled_lock(0);
        for now in 0..u64::from(MATCHES_TO_UNLOCK) {
            lock.update(10_000 + now, seen(0.1));
        }
        lock.start_enrolling(20_000);
        assert_eq!(
            lock.state(),
            State::Enrolling {
                samples: 0,
                next_ms: 20_000
            }
        );
        assert!(!lock.wants_distance());
    }

    #[test]
    fn the_threshold_is_clamped() {
        let mut lock = FaceLock::new(5.0);
        assert_eq!(lock.threshold(), MAX_THRESHOLD);
        lock.set_threshold(0.0);
        assert_eq!(lock.threshold(), MIN_THRESHOLD);
        lock.set_threshold(0.4);
        assert_eq!(lock.threshold(), 0.4);
    }
}
