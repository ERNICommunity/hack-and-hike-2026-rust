//! IMA ADPCM decoding: four bits per sample, decoded one nibble at a time.
//!
//! Sound clips are stored in flash in this format because it is a quarter
//! of the size of 16-bit PCM and decodes with a few integer operations.

const INDEX_TABLE: [i8; 16] = [-1, -1, -1, -1, 2, 4, 6, 8, -1, -1, -1, -1, 2, 4, 6, 8];

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
    predictor: i32,
    step_index: usize,
}

impl Decoder {
    pub const fn new() -> Self {
        Self {
            predictor: 0,
            step_index: 0,
        }
    }

    /// Decode the next 4-bit code (only the low four bits are used).
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

/// All samples of an encoded clip.
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
    fn the_largest_code_moves_the_predictor_by_the_whole_step() {
        let mut decoder = Decoder::new();
        // Step 7: 7/8 + 7/4 + 7/2 + 7 = 0 + 1 + 3 + 7 = 11.
        assert_eq!(decoder.decode(0x7), 11);
        // The step index grew by 8 (step 16): 2 + 4 + 8 + 16 = 30 more.
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
        assert!(decode_all(&[0x88, 0x88, 0x00]).all(|sample| sample == 0));
        assert_eq!(decode_all(&[0x77; 4]).count(), 8);
    }
}
