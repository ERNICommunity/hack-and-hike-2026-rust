//! Microphone PCM capability.
//!
//! The public boundary is a bounded stream of complete interleaved stereo PCM
//! blocks. The private shared audio runtime owns I2S0/DMA and feeds this stream;
//! applications decide how to interpret or visualize the captured sound.

use super::audio;

pub const SAMPLE_RATE_HZ: u32 = audio::SAMPLE_RATE_HZ;
pub const CHANNELS: usize = 2;
pub const FRAMES_PER_BLOCK: usize = 512;
pub const SAMPLES_PER_BLOCK: usize = FRAMES_PER_BLOCK * CHANNELS;
pub const QUEUE_CAPACITY_BLOCKS: usize = 4;

/// Metadata for one complete PCM block.
///
/// `sequence` advances for every captured block. `dropped_blocks` is the
/// cumulative number of oldest queued blocks discarded because the bounded
/// consumer queue was full, so consumers can detect loss both from this counter
/// and from sequence gaps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MicBlockInfo {
    pub sequence: u32,
    pub dropped_blocks: u32,
    pub peak_left: u16,
    pub peak_right: u16,
}

/// CPU0 reader for captured signed 16-bit interleaved stereo PCM.
pub struct Microphone {
    reader: audio::MicReader,
}

impl Microphone {
    pub fn try_read(&mut self, samples: &mut [i16; SAMPLES_PER_BLOCK]) -> Option<MicBlockInfo> {
        self.reader.try_read(samples)
    }
}

pub(crate) fn from_reader(reader: audio::MicReader) -> Microphone {
    Microphone { reader }
}
