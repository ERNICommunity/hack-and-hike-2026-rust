//! IMA ADPCM decoding: four bits per sample, decoded one nibble at a time.
//!
//! ADPCM (adaptive differential pulse-code modulation) stores each sample as
//! a 4-bit change from the previous sample. The size of one step of change
//! adapts to the sound. IMA ADPCM is a common variant of it.
//!
//! Sound clips are stored in flash in this format. It needs a quarter of the
//! space of 16-bit PCM (uncompressed samples), and it decodes with a few
//! integer operations.

/// How far the step index moves after each 4-bit code, indexed by the code.
/// Codes with a small magnitude (0 to 3) make the next step smaller. Codes
/// with a large magnitude (4 to 7) make it bigger. The sign bit (value 8)
/// does not change the step, so the second half of the table repeats the
/// first half.
const INDEX_TABLE: [i8; 16] = [-1, -1, -1, -1, 2, 4, 6, 8, -1, -1, -1, -1, 2, 4, 6, 8];

/// The 89 step sizes of IMA ADPCM, each about 10 % larger than the one
/// before.
///
/// A code with magnitude `m` (0 to 7) changes the previous sample by about
/// `(m + 0.5) / 4` of the current step: from 1/8 of a step for code 0 to
/// 15/8 of a step for code 7. The sign bit says whether to add or subtract.
const STEP_TABLE: [i32; 89] = [
    7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37, 41, 45, 50, 55, 60, 66,
    73, 80, 88, 97, 107, 118, 130, 143, 157, 173, 190, 209, 230, 253, 279, 307, 337, 371, 408, 449,
    494, 544, 598, 658, 724, 796, 876, 963, 1060, 1166, 1282, 1411, 1552, 1707, 1878, 2066, 2272,
    2499, 2749, 3024, 3327, 3660, 4026, 4428, 4871, 5358, 5894, 6484, 7132, 7845, 8630, 9493,
    10442, 11487, 12635, 13899, 15289, 16818, 18500, 20350, 22385, 24623, 27086, 29794, 32767,
];

/// Decoder state. Create one per clip and feed it the nibbles in order, high
/// nibble of each byte first.
#[derive(Clone, Copy, Debug, Default)]
pub struct Decoder {
    /// The last decoded sample. The next code moves it up or down. It always
    /// stays within the `i16` range.
    predictor: i32,
    /// Index into `STEP_TABLE` of the step for the next code, 0 to 88.
    step_index: usize,
}

impl Decoder {
    /// The state at the start of a clip.
    pub const fn new() -> Self {
        Self {
            predictor: 0,
            step_index: 0,
        }
    }

    /// Decode the next 4-bit code and return the new sample. Only the low
    /// four bits of `nibble` are used.
    pub fn decode(&mut self, nibble: u8) -> i16 {
        let nibble = nibble & 0x0F;
        let step = STEP_TABLE[self.step_index];
        let mut difference = step >> 3;
        if nibble & 0x01 != 0 {
            difference += step >> 2;
        }
        if nibble & 0x02 != 0 {
            difference += step >> 1;
        }
        if nibble & 0x04 != 0 {
            difference += step;
        }

        if nibble & 0x08 != 0 {
            self.predictor -= difference;
        } else {
            self.predictor += difference;
        }
        self.predictor = self
            .predictor
            .clamp(i32::from(i16::MIN), i32::from(i16::MAX));

        let index = self.step_index as i32 + i32::from(INDEX_TABLE[usize::from(nibble)]);
        self.step_index = index.clamp(0, STEP_TABLE.len() as i32 - 1) as usize;

        self.predictor as i16
    }
}

/// All samples of an encoded clip: two samples per byte, high nibble first.
pub fn decode_all(encoded: &[u8]) -> impl Iterator<Item = i16> + '_ {
    let mut decoder = Decoder::new();
    encoded
        .iter()
        .flat_map(|byte| [byte >> 4, byte & 0x0F])
        .map(move |nibble| decoder.decode(nibble))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_largest_code_adds_every_fraction_of_the_step() {
        let mut decoder = Decoder::new();
        // Step 7, with the integer divisions rounded down:
        // 7/8 + 7/4 + 7/2 + 7 = 0 + 1 + 3 + 7 = 11.
        assert_eq!(decoder.decode(0x7), 11);
        // The step index grew by 8, to step 16: 2 + 4 + 8 + 16 = 30 more.
        assert_eq!(decoder.decode(0x7), 11 + 30);
    }

    #[test]
    fn the_sign_bit_mirrors_the_code() {
        let mut up = Decoder::new();
        let mut down = Decoder::new();
        for code in [0x3, 0x5, 0x1] {
            assert_eq!(up.decode(code), -down.decode(code | 0x8));
        }
    }

    #[test]
    fn silence_stays_silent() {
        // Codes 0 and 8 change a sample by 1/8 of the smallest step (7),
        // which rounds down to 0.
        assert!(decode_all(&[0x88, 0x88, 0x00]).all(|sample| sample == 0));
        // Every byte holds two samples.
        assert_eq!(decode_all(&[0x77; 4]).count(), 8);
    }
}
