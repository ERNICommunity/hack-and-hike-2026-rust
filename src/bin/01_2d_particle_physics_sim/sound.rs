//! The drag noise: soft filtered noise plus tiny grain clicks. Its volume
//! follows the motion and is exactly 0 when nothing moves.

use hack_and_hike::capabilities::audio::{self, Speaker};

/// Frames generated per `write`. 128 stereo frames = 512 bytes of stack.
const CHUNK_FRAMES: usize = 128;
/// Loudest sample at full volume (i16 goes to 32767). Keeps it gentle.
const MAX_AMPLITUDE: f32 = 6_000.0;
/// Low-pass strength, 0..1: smaller is duller. 0.45 ≈ 1.5 kHz at 16 kHz.
const LOW_PASS: f32 = 0.45;
/// Per-sample volume steps: ~20 ms to rise, ~150 ms to fall at 16 kHz.
const ATTACK: f32 = 0.003;
/// See [`ATTACK`].
const RELEASE: f32 = 0.0004;
/// Below this volume, with no motion, output real silence.
const SILENT: f32 = 0.002;
/// Chance per sample of a grain click at full volume (~1,300 per second).
const CLICK_CHANCE: f32 = 0.08;
/// How fast a click fades, per sample.
const CLICK_DECAY: f32 = 0.9;

/// The noise generator and its smoothed state.
pub struct DragNoise {
    /// Random number generator state (xorshift32, never 0).
    rng: u32,
    /// The low-pass filter's output.
    low: f32,
    /// The current volume, 0..1, moving towards the wanted volume.
    gain: f32,
    /// The current click's loudness, fading to 0.
    click: f32,
}

impl Default for DragNoise {
    fn default() -> Self {
        Self {
            rng: 0x9e37_79b9,
            low: 0.0,
            gain: 0.0,
            click: 0.0,
        }
    }
}

impl DragNoise {
    /// Fill the speaker queue as far as it goes, at `volume` (0..1).
    pub fn fill(&mut self, speaker: &mut Speaker, volume: f32) {
        let mut chunk = [0_i16; CHUNK_FRAMES * audio::CHANNELS];
        loop {
            let frames = speaker.available_frames().min(CHUNK_FRAMES);
            if frames == 0 {
                return;
            }
            let samples = &mut chunk[..frames * audio::CHANNELS];
            for frame in samples.chunks_exact_mut(audio::CHANNELS) {
                // One speaker: the same sample on left and right.
                frame.fill(self.next_sample(volume));
            }
            speaker.write(samples);
        }
    }

    /// One output sample.
    fn next_sample(&mut self, volume: f32) -> i16 {
        // Envelope: move the gain a small step towards the wanted volume.
        let rate = if volume > self.gain { ATTACK } else { RELEASE };
        self.gain += (volume - self.gain) * rate;
        if volume <= 0.0 && self.gain < SILENT {
            self.gain = 0.0;
            return 0;
        }

        // "shhh": white noise through a one-pole low-pass filter.
        let white = self.noise();
        self.low += (white - self.low) * LOW_PASS;

        // Grain clicks: more often when louder, each fades in a few samples.
        self.click *= CLICK_DECAY;
        if self.chance() < self.gain * CLICK_CHANCE {
            self.click = 1.0;
        }

        let mixed = self.low * 0.7 + white * self.click * 0.5;
        (mixed * self.gain * MAX_AMPLITUDE) as i16
    }

    /// Next random 32-bit number (xorshift32).
    fn next_random(&mut self) -> u32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        x
    }

    /// White noise: a random value in -1..1.
    fn noise(&mut self) -> f32 {
        self.next_random() as i32 as f32 / 2_147_483_648.0
    }

    /// A random value in 0..1.
    fn chance(&mut self) -> f32 {
        (self.next_random() >> 8) as f32 / 16_777_216.0
    }
}
