//! Speaker handle.

use super::channels::Service;

/// Application handle for the speaker.
///
/// The application queues interleaved stereo PCM; CPU1 drains the queue into
/// the amplifier and plays silence whenever the queue runs empty. Feed it a
/// little at a time from your main loop instead of blocking on one big write.
pub struct Speaker {
    pub(super) service: &'static Service,
}

impl Speaker {
    /// Stereo frames that can be queued right now.
    ///
    /// Only the application writes to the queue, so a following [`write`] of
    /// at most this many frames is always accepted in full.
    ///
    /// [`write`]: Speaker::write
    pub fn available_frames(&self) -> usize {
        self.service
            .speaker
            .lock(|queue| queue.borrow().free_frames())
    }

    /// Queue interleaved stereo samples (left, right, left, right, ...).
    /// Returns the number of frames accepted; an odd trailing sample is ignored.
    pub fn write(&mut self, samples: &[i16]) -> usize {
        self.service
            .speaker
            .lock(|queue| queue.borrow_mut().write(samples))
    }
}
