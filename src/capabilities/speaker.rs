//! Speaker PCM capability.
//!
//! Applications write signed 16-bit interleaved stereo PCM into a bounded
//! nonblocking queue. The private shared audio runtime owns I2S0/DMA and drains
//! this queue on CPU1, emitting silence whenever no speaker PCM is available.

use super::audio;

pub(crate) const SAMPLE_RATE_HZ: u32 = audio::SAMPLE_RATE_HZ;
pub(crate) const CHANNELS: usize = 2;

/// CPU0 writer for signed 16-bit interleaved stereo PCM.
pub(crate) struct Speaker {
    writer: audio::SpeakerWriter,
}

impl Speaker {
    /// Try to enqueue complete stereo frames without blocking.
    ///
    /// Returns the number of frames accepted. An odd trailing sample is ignored.
    pub(crate) fn try_write_interleaved(&mut self, samples: &[i16]) -> usize {
        self.writer.try_write_interleaved(samples)
    }

    /// Return the number of complete stereo frames that can currently be queued.
    ///
    /// Returns zero while the private queue is momentarily locked by CPU1.
    pub(crate) fn available_frames(&self) -> usize {
        self.writer.available_frames()
    }
}

pub(crate) fn from_writer(writer: audio::SpeakerWriter) -> Speaker {
    Speaker { writer }
}
