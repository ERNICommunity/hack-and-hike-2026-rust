//! The grains and the box: the "pixel dust" simulation.
//!
//! The box is a grid of cells. Each cell is empty or holds exactly one
//! grain. A grain has a position and a velocity in cells (floats). Each step
//! it speeds up with gravity and tries to move:
//!
//! 1. Inside its own cell, or into an empty cell: just move.
//! 2. Blocked: hop to the free neighbour cell that is most downhill (sand
//!    only takes cells that are clearly downhill, so it keeps a slope).
//! 3. Water only: flow sideways towards a drop a few cells away, so its
//!    surface ends up level.
//! 4. Nowhere to go: bounce back.
//!
//! Every hop goes downhill, so the grains always come to rest.

use core::f32::consts::FRAC_1_SQRT_2;

use alloc::vec;

/// Width of the box, in cells.
pub const COLUMNS: usize = 80;
/// Height of the box, in cells.
pub const ROWS: usize = 56;
/// A grid value for "no grain here". Other values are grain index + 1.
const EMPTY: u16 = 0;
/// The 8 neighbour cells, as `(dx, dy)` offsets.
const NEIGHBOURS: [(i32, i32); 8] = [
    (1, 0),
    (1, 1),
    (0, 1),
    (-1, 1),
    (-1, 0),
    (-1, -1),
    (0, -1),
    (1, -1),
];
/// Water moves at most this many cells per step when it flows sideways.
const FLOW_HOP: i32 = 3;
/// Speeds (cells/s, squared) where the colour gets one step brighter.
const SPEED_LEVELS_SQUARED: [f32; 3] = [10.0 * 10.0, 30.0 * 30.0, 60.0 * 60.0];

/// One grain: position (cells, 0.0 = left/top edge) and velocity (cells/s).
#[derive(Clone, Copy)]
struct Grain {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
}

/// Which way is down, for one step.
#[derive(Clone, Copy)]
struct Pull {
    /// Gravity direction, length 1.
    unit: [f32; 2],
    /// The neighbour cell that points most downhill.
    cell: (i32, i32),
}

/// What the grains are made of. Same code, different numbers.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Material {
    Sand,
    Water,
}

impl Material {
    /// Fraction of the speed lost per second (air and friction).
    fn drag(self) -> f32 {
        match self {
            Material::Sand => 1.5,
            Material::Water => 0.3,
        }
    }

    /// Fraction of the speed kept after a bounce (restitution).
    fn bounce(self) -> f32 {
        match self {
            Material::Sand => 0.1,
            Material::Water => 0.25,
        }
    }

    /// Fraction of the speed kept when hopping around another grain.
    fn slide(self) -> f32 {
        match self {
            Material::Sand => 0.5,
            Material::Water => 0.95,
        }
    }

    /// How downhill a blocked grain's hop must be: the cosine of the angle
    /// between the hop and gravity. Sand 0.17 (within ~80°) keeps slopes;
    /// water 0.05 takes almost anything downhill.
    fn min_downhill(self) -> f32 {
        match self {
            Material::Sand => 0.17,
            Material::Water => 0.05,
        }
    }

    /// How far (cells) a grain looks sideways for a drop. 0 = never flows.
    fn flow_look(self) -> i32 {
        match self {
            Material::Sand => 0,
            Material::Water => 8,
        }
    }

    /// The name for the status line.
    pub fn name(self) -> &'static str {
        match self {
            Material::Sand => "sand",
            Material::Water => "water",
        }
    }

    /// The other material.
    pub fn toggled(self) -> Self {
        match self {
            Material::Sand => Material::Water,
            Material::Water => Material::Sand,
        }
    }
}

/// All grains, the grid, and the current material.
pub struct World {
    /// Every grain. Lives on the heap in internal RAM (too big for the stack).
    grains: &'static mut [Grain],
    /// `COLUMNS * ROWS` cells, row by row: `EMPTY` or grain index + 1.
    grid: &'static mut [u16],
    /// State of the random number generator (xorshift32, never 0).
    rng: u32,
    /// What the grains behave like.
    pub material: Material,
}

impl World {
    /// A box with `count` grains, packed into the top rows, at rest.
    pub fn new(count: usize) -> Self {
        assert!(count < COLUMNS * ROWS, "the grains must fit in the box");
        // Internal RAM (the normal heap), not PSRAM: every step reads all
        // grains and many cells, and PSRAM is much slower. ~28 KB in total.
        let empty = Grain {
            x: 0.0,
            y: 0.0,
            vx: 0.0,
            vy: 0.0,
        };
        let grains = vec![empty; count].leak();
        let grid = vec![EMPTY; COLUMNS * ROWS].leak();
        for (i, grain) in grains.iter_mut().enumerate() {
            let (cx, cy) = (i % COLUMNS, i / COLUMNS);
            grain.x = cx as f32 + 0.5;
            grain.y = cy as f32 + 0.5;
            grid[cy * COLUMNS + cx] = (i + 1) as u16;
        }
        Self {
            grains,
            grid,
            rng: 0x1234_5678,
            material: Material::Sand,
        }
    }

    /// Move every grain forward by `dt` seconds under `gravity` (cells/s²,
    /// `[right, down]`). Return how many grains changed cell: the motion.
    pub fn step(&mut self, gravity: [f32; 2], dt: f32) -> u32 {
        let keep = 1.0 - self.material.drag() * dt;
        // Less than one cell per step, so a grain never jumps over another.
        let max_speed = 0.95 / dt;
        let max_speed_squared = max_speed * max_speed;
        let pull = pull_of(gravity);
        let mut moved = 0;
        for index in 0..self.grains.len() {
            let mut grain = self.grains[index];
            // Semi-implicit Euler: velocity first, then position.
            grain.vx = (grain.vx + gravity[0] * dt) * keep;
            grain.vy = (grain.vy + gravity[1] * dt) * keep;
            // Compare squares: most grains are slower, and skip the sqrt.
            let speed_squared = grain.vx * grain.vx + grain.vy * grain.vy;
            if speed_squared > max_speed_squared {
                let speed = libm::sqrtf(speed_squared);
                grain.vx *= max_speed / speed;
                grain.vy *= max_speed / speed;
            }
            if self.advance(index, &mut grain, dt, pull) {
                moved += 1;
            }
            self.grains[index] = grain;
        }
        moved
    }

    /// Move one grain by its velocity, handling collisions. Return whether
    /// it changed cell.
    fn advance(&mut self, index: usize, grain: &mut Grain, dt: f32, pull: Option<Pull>) -> bool {
        let from = cell_of(grain.x, grain.y);
        let (nx, ny) = (grain.x + grain.vx * dt, grain.y + grain.vy * dt);
        let to = cell_of(nx, ny);

        // 1. Same cell (always free), or an empty cell: just move.
        if to == from || self.is_free(to.0, to.1) {
            if to != from {
                self.move_grain(index, from, to);
            }
            (grain.x, grain.y) = (nx, ny);
            return to != from;
        }

        if let Some(pull) = pull {
            // A random sign, to choose fairly between mirror-image cells.
            let tie = if self.random_bit() { 1 } else { -1 };

            // 2. Hop to the best free neighbour that is downhill enough.
            if let Some(offset) = self.best_hop(from, grain, pull, tie) {
                self.hop(index, grain, from, offset, self.material.slide());
                return true;
            }
            // 3. Water: flow sideways towards a drop.
            if let Some(offset) = self.flow(from, pull, tie) {
                self.hop(index, grain, from, offset, 1.0);
                return true;
            }
        }

        // 4. Nowhere to go: bounce back and stay in this cell.
        let bounce = self.material.bounce();
        grain.vx = -grain.vx * bounce;
        grain.vy = -grain.vy * bounce;
        false
    }

    /// The free neighbour of `from` that is downhill enough, preferring the
    /// most downhill one, then the one closest to the grain's motion.
    fn best_hop(
        &self,
        from: (i32, i32),
        grain: &Grain,
        pull: Pull,
        tie: i32,
    ) -> Option<(i32, i32)> {
        let speed = libm::hypotf(grain.vx, grain.vy).max(1e-6);
        let motion = [grain.vx / speed, grain.vy / speed];
        let mut best = None;
        let mut best_score = f32::MIN;
        for offset in NEIGHBOURS {
            if !self.is_free(from.0 + offset.0, from.1 + offset.1) {
                continue;
            }
            let downhill = alignment(offset, pull.unit);
            if downhill <= self.material.min_downhill() {
                continue;
            }
            let score = 2.0 * downhill
                + alignment(offset, motion)
                + 0.001 * (tie * (offset.0 - offset.1)) as f32;
            if score > best_score {
                best_score = score;
                best = Some(offset);
            }
        }
        best
    }

    /// Water only: look sideways (across gravity) for a free cell with a
    /// free cell below it, that is lower than here. Return a hop of up to
    /// [`FLOW_HOP`] cells towards it.
    fn flow(&self, from: (i32, i32), pull: Pull, tie: i32) -> Option<(i32, i32)> {
        let look = self.material.flow_look();
        let (dx, dy) = pull.cell;
        let side = (-dy, dx);
        // How much lower (along gravity) one cell down or sideways is.
        let drop_down = dx as f32 * pull.unit[0] + dy as f32 * pull.unit[1];
        let drop_side = side.0 as f32 * pull.unit[0] + side.1 as f32 * pull.unit[1];
        for sign in [tie, -tie] {
            let (ox, oy) = (side.0 * sign, side.1 * sign);
            for k in 1..=look {
                let (px, py) = (from.0 + ox * k, from.1 + oy * k);
                if !self.is_free(px, py) {
                    break;
                }
                let lower = (k * sign) as f32 * drop_side + drop_down > 0.05;
                if lower && self.is_free(px + dx, py + dy) {
                    let hop = k.min(FLOW_HOP);
                    return Some((ox * hop, oy * hop));
                }
            }
        }
        None
    }

    /// Move grain `index` by `offset` cells, keeping where it sits inside
    /// the cell (so it can go on at once), and keep `keep` of its speed.
    fn hop(
        &mut self,
        index: usize,
        grain: &mut Grain,
        from: (i32, i32),
        offset: (i32, i32),
        keep: f32,
    ) {
        self.move_grain(index, from, (from.0 + offset.0, from.1 + offset.1));
        grain.x += offset.0 as f32;
        grain.y += offset.1 as f32;
        grain.vx *= keep;
        grain.vy *= keep;
    }

    /// Whether `(cx, cy)` is inside the box and empty.
    fn is_free(&self, cx: i32, cy: i32) -> bool {
        (0..COLUMNS as i32).contains(&cx)
            && (0..ROWS as i32).contains(&cy)
            && self.grid[cy as usize * COLUMNS + cx as usize] == EMPTY
    }

    /// Update the grid: grain `index` leaves `from` and enters `to`.
    fn move_grain(&mut self, index: usize, from: (i32, i32), to: (i32, i32)) {
        self.grid[from.1 as usize * COLUMNS + from.0 as usize] = EMPTY;
        self.grid[to.1 as usize * COLUMNS + to.0 as usize] = (index + 1) as u16;
    }

    /// A random true or false (xorshift32).
    fn random_bit(&mut self) -> bool {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        x & 1 == 1
    }

    /// For drawing: `None` for an empty cell, otherwise how fast its grain
    /// moves, from 0 (slow) to 3 (fast).
    pub fn speed_level(&self, cx: usize, cy: usize) -> Option<usize> {
        let value = self.grid[cy * COLUMNS + cx];
        if value == EMPTY {
            return None;
        }
        let grain = self.grains[usize::from(value) - 1];
        let speed_squared = grain.vx * grain.vx + grain.vy * grain.vy;
        Some(
            SPEED_LEVELS_SQUARED
                .iter()
                .filter(|&&level| speed_squared > level)
                .count(),
        )
    }
}

/// Which way is down, or `None` when there is no gravity (board flat).
fn pull_of(gravity: [f32; 2]) -> Option<Pull> {
    let length = libm::hypotf(gravity[0], gravity[1]);
    if length <= 0.0 {
        return None;
    }
    let unit = [gravity[0] / length, gravity[1] / length];
    let mut cell = NEIGHBOURS[0];
    for offset in NEIGHBOURS {
        if alignment(offset, unit) > alignment(cell, unit) {
            cell = offset;
        }
    }
    Some(Pull { unit, cell })
}

/// Cosine of the angle between a neighbour `offset` and the unit vector
/// `direction`: 1 = same way, 0 = across, -1 = opposite.
fn alignment(offset: (i32, i32), direction: [f32; 2]) -> f32 {
    let length = if offset.0 != 0 && offset.1 != 0 {
        FRAC_1_SQRT_2
    } else {
        1.0
    };
    (offset.0 as f32 * direction[0] + offset.1 as f32 * direction[1]) * length
}

/// The cell that contains the point `(x, y)`. `floorf`, not `as`, so that
/// -0.3 gives -1 (outside), not 0.
fn cell_of(x: f32, y: f32) -> (i32, i32) {
    (libm::floorf(x) as i32, libm::floorf(y) as i32)
}
