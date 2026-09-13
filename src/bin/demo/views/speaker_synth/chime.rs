//! Flash-resident one-shot decoded incrementally from IMA ADPCM.
//!
//! `assets/speaker_chime.adpcm` is derived offline from the user-supplied MP3:
//! stereo 48 kHz MP3 -> mono 16 kHz signed-16 PCM -> IMA ADPCM. The resulting
//! 11,904 samples are about 744 ms at the firmware's native 16 kHz rate.
//! Keeping only the ADPCM in flash avoids a runtime MP3 decoder and keeps
//! playback bounded and allocation-free.

const DATA: &[u8] = include_bytes!("../../../../../assets/speaker_chime.adpcm");
const CHIME_SAMPLES: usize = 11_904;
const _: () = assert!(DATA.len() * 2 == CHIME_SAMPLES);

const INDEX_TABLE: [i8; 16] = [-1, -1, -1, -1, 2, 4, 6, 8, -1, -1, -1, -1, 2, 4, 6, 8];

const STEP_TABLE: [i32; 89] = [
    7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37, 41, 45, 50, 55, 60, 66,
    73, 80, 88, 97, 107, 118, 130, 143, 157, 173, 190, 209, 230, 253, 279, 307, 337, 371, 408, 449,
    494, 544, 598, 658, 724, 796, 876, 963, 1060, 1166, 1282, 1411, 1552, 1707, 1878, 2066, 2272,
    2499, 2749, 3024, 3327, 3660, 4026, 4428, 4871, 5358, 5894, 6484, 7132, 7845, 8630, 9493,
    10442, 11487, 12635, 13899, 15289, 16818, 18500, 20350, 22385, 24623, 27086, 29794, 32767,
];

pub(super) struct FlashChime {
    byte_index: usize,
    current_byte: u8,
    high_nibble: bool,
    predictor: i32,
    step_index: i32,
    playing: bool,
}

impl FlashChime {
    pub(super) const fn new() -> Self {
        Self {
            byte_index: 0,
            current_byte: 0,
            high_nibble: true,
            predictor: 0,
            step_index: 0,
            playing: false,
        }
    }

    pub(super) fn restart(&mut self) {
        self.byte_index = 0;
        self.current_byte = 0;
        self.high_nibble = true;
        self.predictor = 0;
        self.step_index = 0;
        self.playing = true;
    }

    pub(super) fn is_playing(&self) -> bool {
        self.playing
    }

    pub(super) fn next_sample(&mut self) -> i16 {
        if !self.playing {
            return 0;
        }

        let nibble = if self.high_nibble {
            let Some(&encoded) = DATA.get(self.byte_index) else {
                self.playing = false;
                return 0;
            };
            self.byte_index += 1;
            self.current_byte = encoded;
            self.high_nibble = false;
            encoded >> 4
        } else {
            self.high_nibble = true;
            self.current_byte & 0x0F
        };

        self.decode_nibble(nibble)
    }

    fn decode_nibble(&mut self, nibble: u8) -> i16 {
        let step = STEP_TABLE[self.step_index as usize];
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

        self.step_index += i32::from(INDEX_TABLE[nibble as usize]);
        self.step_index = self.step_index.clamp(0, 88);

        self.predictor as i16
    }
}
