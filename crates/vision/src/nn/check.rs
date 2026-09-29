//! A check of the two networks on the board against the computer.
//!
//! Both networks run on made-up inputs that are the same on every
//! machine ([`noise`]), and a fingerprint of their outputs
//! ([`fingerprint`]) must be the one the computer computes with the same
//! weights: [`DETECTOR`] and [`RECOGNIZER`]. The board runs the vector
//! unit's assembly and the computer the scalar model of it, so a
//! difference means that the two do not compute the same numbers any
//! more. The application runs the check when it starts, before the camera
//! does, which also times each network alone.
//!
//! `tests/fingerprints.rs` checks the two constants against the weights
//! in `assets/models`. A change to a network's numbers on purpose (other
//! weights, another kernel) changes them: that test prints the new ones.

/// The seed of the detector's input.
pub const DETECTOR_SEED: u32 = 1;
/// The seed of the recognizer's input.
pub const RECOGNIZER_SEED: u32 = 2;
/// The fingerprint of the detector's heads (every `cls`, `obj`, `bbox`
/// and `kps` value of the three levels, in that order) on
/// `noise(DETECTOR_SEED)`.
pub const DETECTOR: u64 = 0x2809_abed_1200_e534;
/// The fingerprint of the recognizer's raw embedding on
/// `noise(RECOGNIZER_SEED)`.
pub const RECOGNIZER: u64 = 0x7349_3343_282a_6a08;

/// Fill `values` with numbers that look random, the same for the same
/// `seed` on every machine: a linear congruential generator.
pub fn noise(seed: u32, values: &mut [i8]) {
    let mut state = seed.wrapping_mul(2_654_435_761).wrapping_add(1);
    for value in values {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        *value = (state >> 24) as u8 as i8;
    }
}

/// FNV-1a over the bits of `values`: any changed bit changes it.
pub fn fingerprint(values: impl IntoIterator<Item = f32>) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for value in values {
        for byte in value.to_bits().to_le_bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    hash
}
