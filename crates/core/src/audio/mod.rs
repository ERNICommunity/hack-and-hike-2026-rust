//! Audio building blocks with no hardware dependency.
//!
//! - [`FrameRing`]: the queue between an application and the speaker.
//! - [`adpcm`]: decoding of compressed sound clips.

pub mod adpcm;

/// A ring buffer of interleaved audio frames, `CHANNELS` samples each.
///
/// Only whole frames are written and read. So a sample of one channel can
/// never end up in the place of another channel.
pub struct FrameRing<const SAMPLES: usize, const CHANNELS: usize> {
    /// Storage of the ring. The queued samples start at `read_index`. When
    /// they reach the end of the array, they continue at its start.
    samples: [i16; SAMPLES],
    /// Index of the oldest unread sample in `samples`.
    read_index: usize,
    /// Samples (not frames) waiting to be read. Always a multiple of
    /// `CHANNELS`.
    len: usize,
}

impl<const SAMPLES: usize, const CHANNELS: usize> Default for FrameRing<SAMPLES, CHANNELS> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const SAMPLES: usize, const CHANNELS: usize> FrameRing<SAMPLES, CHANNELS> {
    /// An empty ring.
    ///
    /// # Panics
    ///
    /// When `CHANNELS` is zero or `SAMPLES` is not a multiple of `CHANNELS`.
    /// In a const context, such as a `static`, the error appears at compile
    /// time. Otherwise it is a panic at run time.
    pub const fn new() -> Self {
        assert!(CHANNELS > 0 && SAMPLES.is_multiple_of(CHANNELS));
        Self {
            samples: [0; SAMPLES],
            read_index: 0,
            len: 0,
        }
    }

    /// Frames the ring holds when full.
    pub const fn capacity_frames() -> usize {
        SAMPLES / CHANNELS
    }

    /// Frames waiting to be read.
    pub const fn frames(&self) -> usize {
        self.len / CHANNELS
    }

    /// Frames that can be written before the ring is full.
    pub const fn free_frames(&self) -> usize {
        (SAMPLES - self.len) / CHANNELS
    }

    /// Append complete frames from `samples`. Returns how many frames were
    /// accepted.
    ///
    /// An incomplete frame at the end of `samples` is ignored. When the ring
    /// has no room for all frames, the frames that do not fit are not
    /// written.
    pub fn write(&mut self, samples: &[i16]) -> usize {
        let frames = (samples.len() / CHANNELS).min(self.free_frames());
        let count = frames * CHANNELS;
        let write_index = (self.read_index + self.len) % SAMPLES;
        let first = count.min(SAMPLES - write_index);

        self.samples[write_index..write_index + first].copy_from_slice(&samples[..first]);
        self.samples[..count - first].copy_from_slice(&samples[first..count]);
        self.len += count;
        frames
    }

    /// Move complete frames into `out`, as many as fit and are queued.
    /// Returns how many frames were read.
    pub fn read(&mut self, out: &mut [i16]) -> usize {
        let frames = (out.len() / CHANNELS).min(self.frames());
        let count = frames * CHANNELS;
        let first = count.min(SAMPLES - self.read_index);

        out[..first].copy_from_slice(&self.samples[self.read_index..self.read_index + first]);
        out[first..count].copy_from_slice(&self.samples[..count - first]);
        self.read_index = (self.read_index + count) % SAMPLES;
        self.len -= count;
        frames
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type Stereo6 = FrameRing<12, 2>;

    #[test]
    fn frames_come_out_in_order_and_wrap_around() {
        let mut ring = Stereo6::new();
        assert_eq!(Stereo6::capacity_frames(), 6);

        assert_eq!(ring.write(&[1, 2, 3, 4, 5, 6, 7, 8]), 4);
        let mut out = [0; 6];
        assert_eq!(ring.read(&mut out), 3);
        assert_eq!(out, [1, 2, 3, 4, 5, 6]);

        // The next write crosses the end of the storage.
        assert_eq!(ring.write(&[9, 10, 11, 12, 13, 14, 15, 16]), 4);
        assert_eq!(ring.frames(), 5);
        let mut out = [0; 10];
        assert_eq!(ring.read(&mut out), 5);
        assert_eq!(out, [7, 8, 9, 10, 11, 12, 13, 14, 15, 16]);
        assert_eq!(ring.frames(), 0);
    }

    #[test]
    fn a_full_ring_accepts_nothing_more() {
        let mut ring = Stereo6::new();
        assert_eq!(ring.write(&[0; 12]), 6);
        assert_eq!(ring.free_frames(), 0);
        assert_eq!(ring.write(&[1, 1]), 0);
    }

    #[test]
    fn an_odd_trailing_sample_is_ignored() {
        let mut ring = Stereo6::new();
        assert_eq!(ring.write(&[1, 2, 3]), 1);
        let mut out = [0; 4];
        assert_eq!(ring.read(&mut out), 1);
        assert_eq!(out, [1, 2, 0, 0]);
    }
}
