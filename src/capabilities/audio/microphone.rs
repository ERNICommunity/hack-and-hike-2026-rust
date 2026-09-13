//! Microphone handle.

use super::{SAMPLES_PER_BLOCK, channels::Service};

/// Metadata delivered with every microphone block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MicBlockInfo {
    /// Increments with every captured block, so gaps reveal lost blocks.
    pub sequence: u32,
    /// Total blocks dropped so far because the application fell behind.
    pub dropped_blocks: u32,
    /// Largest absolute left sample in this block.
    pub peak_left: u16,
    /// Largest absolute right sample in this block.
    pub peak_right: u16,
}

/// Application handle for the microphone.
///
/// Audio arrives in blocks of [`super::FRAMES_PER_BLOCK`] stereo frames. Blocks
/// wait in a small queue; when the application does not read them in time the
/// oldest block is dropped and `dropped_blocks` counts it.
pub struct Microphone {
    pub(super) service: &'static Service,
}

impl Microphone {
    /// Copy the oldest unread block into `samples`. `None` when no complete
    /// block is waiting.
    pub fn next_block(&mut self, samples: &mut [i16; SAMPLES_PER_BLOCK]) -> Option<MicBlockInfo> {
        let block = self.service.mic_blocks.try_receive().ok()?;
        samples.copy_from_slice(&block.samples);
        Some(block.info)
    }
}
