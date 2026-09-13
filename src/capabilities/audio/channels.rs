//! Cross-core queues between the application handles (CPU0) and the audio
//! runtime (CPU1).

use core::cell::RefCell;

use embassy_sync::{
    blocking_mutex::{Mutex, raw::CriticalSectionRawMutex},
    channel::Channel,
};
use log::debug;

use super::{
    CHANNELS, SAMPLES_PER_BLOCK,
    microphone::{MicBlockInfo, Microphone},
    speaker::Speaker,
};

/// Microphone blocks the application may fall behind by before the oldest
/// unread block is dropped.
const MIC_QUEUE_BLOCKS: usize = 4;
/// Speaker frames buffered ahead of playback. One DMA descriptor refill takes
/// up to 1023 frames, so the queue holds at least that much to keep a refill
/// from draining it before the application tops it up.
const SPEAKER_QUEUE_FRAMES: usize = 1_024;
const SPEAKER_QUEUE_SAMPLES: usize = SPEAKER_QUEUE_FRAMES * CHANNELS;

/// One captured microphone block as it travels from CPU1 to CPU0.
#[derive(Clone, Copy)]
pub(super) struct MicBlock {
    pub(super) samples: [i16; SAMPLES_PER_BLOCK],
    pub(super) info: MicBlockInfo,
}

impl MicBlock {
    pub(super) const SILENT: Self = Self {
        samples: [0; SAMPLES_PER_BLOCK],
        info: MicBlockInfo {
            sequence: 0,
            dropped_blocks: 0,
            peak_left: 0,
            peak_right: 0,
        },
    };
}

/// Ring of interleaved stereo samples written by CPU0 and drained by CPU1.
pub(super) struct SpeakerQueue {
    samples: [i16; SPEAKER_QUEUE_SAMPLES],
    read_index: usize,
    len: usize,
}

impl SpeakerQueue {
    const fn new() -> Self {
        Self {
            samples: [0; SPEAKER_QUEUE_SAMPLES],
            read_index: 0,
            len: 0,
        }
    }

    pub(super) fn free_frames(&self) -> usize {
        (SPEAKER_QUEUE_SAMPLES - self.len) / CHANNELS
    }

    /// Append complete frames. Returns how many frames were accepted.
    pub(super) fn write(&mut self, samples: &[i16]) -> usize {
        let frames = (samples.len() / CHANNELS).min(self.free_frames());
        let count = frames * CHANNELS;
        let write_index = (self.read_index + self.len) % SPEAKER_QUEUE_SAMPLES;
        let first = count.min(SPEAKER_QUEUE_SAMPLES - write_index);

        self.samples[write_index..write_index + first].copy_from_slice(&samples[..first]);
        self.samples[..count - first].copy_from_slice(&samples[first..count]);
        self.len += count;
        frames
    }

    /// Remove complete frames into `out`. Returns how many frames were read.
    pub(super) fn read(&mut self, out: &mut [i16]) -> usize {
        let frames = (out.len() / CHANNELS).min(self.len / CHANNELS);
        let count = frames * CHANNELS;
        let first = count.min(SPEAKER_QUEUE_SAMPLES - self.read_index);

        out[..first].copy_from_slice(&self.samples[self.read_index..self.read_index + first]);
        out[first..count].copy_from_slice(&self.samples[..count - first]);
        self.read_index = (self.read_index + count) % SPEAKER_QUEUE_SAMPLES;
        self.len -= count;
        frames
    }
}

pub(super) struct Service {
    pub(super) mic_blocks: Channel<CriticalSectionRawMutex, MicBlock, MIC_QUEUE_BLOCKS>,
    pub(super) speaker: Mutex<CriticalSectionRawMutex, RefCell<SpeakerQueue>>,
}

static SERVICE: Service = Service {
    mic_blocks: Channel::new(),
    speaker: Mutex::new(RefCell::new(SpeakerQueue::new())),
};

#[derive(Clone, Copy)]
pub(crate) struct Runtime {
    service: &'static Service,
}

impl Runtime {
    /// True when publishing another block would drop the oldest unread one.
    pub(super) fn microphone_queue_is_full(self) -> bool {
        self.service.mic_blocks.is_full()
    }

    /// Queue a captured block for the application, dropping the oldest unread
    /// block first when the application has fallen behind.
    pub(super) fn publish_microphone_block(self, block: MicBlock) {
        if self.service.mic_blocks.is_full() {
            let _dropped = self.service.mic_blocks.try_receive();
        }
        if self.service.mic_blocks.try_send(block).is_err() {
            debug!("Microphone block lost: queue still full");
        }
    }

    /// Take queued speaker frames for playback. Returns how many frames were read.
    pub(super) fn read_speaker(self, out: &mut [i16]) -> usize {
        self.service
            .speaker
            .lock(|queue| queue.borrow_mut().read(out))
    }
}

pub(crate) struct Endpoints {
    pub(crate) microphone: Microphone,
    pub(crate) speaker: Speaker,
    pub(crate) runtime: Runtime,
}

pub(crate) fn endpoints() -> Endpoints {
    Endpoints {
        microphone: Microphone { service: &SERVICE },
        speaker: Speaker { service: &SERVICE },
        runtime: Runtime { service: &SERVICE },
    }
}
