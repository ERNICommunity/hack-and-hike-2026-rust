//! Cross-core synchronization for the private shared audio runtime.

#[cfg(any(feature = "mic", feature = "speaker"))]
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, mutex::Mutex};
use static_cell::StaticCell;

#[cfg(feature = "mic")]
use crate::capabilities::mic::{MicBlockInfo, QUEUE_CAPACITY_BLOCKS, SAMPLES_PER_BLOCK};

#[cfg(feature = "speaker")]
const SPEAKER_CHANNELS: usize = 2;
#[cfg(feature = "speaker")]
// Buffer one complete DMA descriptor refill. esp-hal's default DMA descriptor
// payload is 4092 bytes; 1024 stereo i16 frames are 4096 bytes. The old
// 512-frame queue was smaller than one refill burst, so playback could drain it
// and zero-pad the remainder before CPU0 generated more PCM.
const SPEAKER_QUEUE_CAPACITY_FRAMES: usize = 1_024;
#[cfg(feature = "speaker")]
const SPEAKER_QUEUE_CAPACITY_SAMPLES: usize = SPEAKER_QUEUE_CAPACITY_FRAMES * SPEAKER_CHANNELS;

#[cfg(feature = "mic")]
#[derive(Clone, Copy)]
struct MicQueueBlock {
    samples: [i16; SAMPLES_PER_BLOCK],
    info: MicBlockInfo,
}

#[cfg(feature = "mic")]
impl MicQueueBlock {
    const EMPTY: Self = Self {
        samples: [0; SAMPLES_PER_BLOCK],
        info: MicBlockInfo {
            sequence: 0,
            dropped_blocks: 0,
            peak_left: 0,
            peak_right: 0,
        },
    };
}

#[cfg(feature = "mic")]
struct MicQueue {
    blocks: [MicQueueBlock; QUEUE_CAPACITY_BLOCKS],
    read_index: usize,
    len: usize,
    next_sequence: u32,
    dropped_blocks: u32,
}

#[cfg(feature = "mic")]
impl MicQueue {
    const fn new() -> Self {
        Self {
            blocks: [MicQueueBlock::EMPTY; QUEUE_CAPACITY_BLOCKS],
            read_index: 0,
            len: 0,
            next_sequence: 0,
            dropped_blocks: 0,
        }
    }

    fn push(&mut self, samples: &[i16; SAMPLES_PER_BLOCK], peak_left: u16, peak_right: u16) -> u32 {
        self.next_sequence = self.next_sequence.wrapping_add(1);

        if self.len == QUEUE_CAPACITY_BLOCKS {
            self.read_index = (self.read_index + 1) % QUEUE_CAPACITY_BLOCKS;
            self.len -= 1;
            self.dropped_blocks = self.dropped_blocks.wrapping_add(1);
        }

        let write_index = (self.read_index + self.len) % QUEUE_CAPACITY_BLOCKS;
        self.blocks[write_index].samples.copy_from_slice(samples);
        self.blocks[write_index].info = MicBlockInfo {
            sequence: self.next_sequence,
            dropped_blocks: self.dropped_blocks,
            peak_left,
            peak_right,
        };
        self.len += 1;
        self.next_sequence
    }

    fn pop(&mut self, out: &mut [i16; SAMPLES_PER_BLOCK]) -> Option<MicBlockInfo> {
        if self.len == 0 {
            return None;
        }

        let index = self.read_index;
        out.copy_from_slice(&self.blocks[index].samples);
        let info = self.blocks[index].info;
        self.read_index = (self.read_index + 1) % QUEUE_CAPACITY_BLOCKS;
        self.len -= 1;
        Some(info)
    }
}

#[cfg(feature = "speaker")]
struct SpeakerQueue {
    samples: [i16; SPEAKER_QUEUE_CAPACITY_SAMPLES],
    read_index: usize,
    len_samples: usize,
}

#[cfg(feature = "speaker")]
impl SpeakerQueue {
    const fn new() -> Self {
        Self {
            samples: [0; SPEAKER_QUEUE_CAPACITY_SAMPLES],
            read_index: 0,
            len_samples: 0,
        }
    }

    fn available_frames(&self) -> usize {
        (SPEAKER_QUEUE_CAPACITY_SAMPLES - self.len_samples) / SPEAKER_CHANNELS
    }

    fn write_interleaved(&mut self, samples: &[i16]) -> usize {
        let frames = (samples.len() / SPEAKER_CHANNELS).min(self.available_frames());
        let sample_count = frames * SPEAKER_CHANNELS;
        let write_index = (self.read_index + self.len_samples) % SPEAKER_QUEUE_CAPACITY_SAMPLES;

        for (offset, sample) in samples[..sample_count].iter().copied().enumerate() {
            self.samples[(write_index + offset) % SPEAKER_QUEUE_CAPACITY_SAMPLES] = sample;
        }
        self.len_samples += sample_count;
        frames
    }

    fn read_interleaved(&mut self, out: &mut [i16]) -> usize {
        let frames = (out.len() / SPEAKER_CHANNELS).min(self.len_samples / SPEAKER_CHANNELS);
        let sample_count = frames * SPEAKER_CHANNELS;

        for (offset, sample) in out[..sample_count].iter_mut().enumerate() {
            *sample = self.samples[(self.read_index + offset) % SPEAKER_QUEUE_CAPACITY_SAMPLES];
        }
        self.read_index = (self.read_index + sample_count) % SPEAKER_QUEUE_CAPACITY_SAMPLES;
        self.len_samples -= sample_count;
        frames
    }
}

#[cfg(feature = "mic")]
type MicQueueStore = Mutex<CriticalSectionRawMutex, MicQueue>;
#[cfg(feature = "speaker")]
type SpeakerQueueStore = Mutex<CriticalSectionRawMutex, SpeakerQueue>;

struct Service {
    #[cfg(feature = "mic")]
    mic_queue: MicQueueStore,
    #[cfg(feature = "speaker")]
    speaker_queue: SpeakerQueueStore,
}

impl Service {
    const fn new() -> Self {
        Self {
            #[cfg(feature = "mic")]
            mic_queue: Mutex::new(MicQueue::new()),
            #[cfg(feature = "speaker")]
            speaker_queue: Mutex::new(SpeakerQueue::new()),
        }
    }
}

static SERVICE: StaticCell<Service> = StaticCell::new();

#[derive(Clone, Copy)]
pub(crate) struct Runtime {
    service: &'static Service,
}

/// Private CPU0 endpoint wrapped by the public `mic::Microphone` capability.
#[cfg(feature = "mic")]
pub(crate) struct MicReader {
    service: &'static Service,
}

/// Private CPU0 endpoint wrapped by the public `speaker::Speaker` capability.
#[cfg(feature = "speaker")]
pub(crate) struct SpeakerWriter {
    service: &'static Service,
}

pub(crate) struct Endpoints {
    pub(crate) runtime: Runtime,
    #[cfg(feature = "mic")]
    pub(crate) mic: MicReader,
    #[cfg(feature = "speaker")]
    pub(crate) speaker: SpeakerWriter,
}

pub(crate) fn init_endpoints() -> Endpoints {
    // `Service` contains the speaker PCM ring and can be several KiB. Construct it
    // directly in the `StaticCell` so bootstrap does not need a same-sized stack
    // temporary before moving the value into static storage.
    let service: &'static Service = SERVICE.init_with(Service::new);
    Endpoints {
        runtime: Runtime { service },
        #[cfg(feature = "mic")]
        mic: MicReader { service },
        #[cfg(feature = "speaker")]
        speaker: SpeakerWriter { service },
    }
}

#[cfg(feature = "mic")]
impl MicReader {
    pub(crate) fn try_read(&mut self, out: &mut [i16; SAMPLES_PER_BLOCK]) -> Option<MicBlockInfo> {
        let mut queue = self.service.mic_queue.try_lock().ok()?;
        queue.pop(out)
    }
}

#[cfg(feature = "speaker")]
impl SpeakerWriter {
    pub(crate) fn try_write_interleaved(&mut self, samples: &[i16]) -> usize {
        let Ok(mut queue) = self.service.speaker_queue.try_lock() else {
            return 0;
        };
        queue.write_interleaved(samples)
    }

    pub(crate) fn available_frames(&self) -> usize {
        let Ok(queue) = self.service.speaker_queue.try_lock() else {
            return 0;
        };
        queue.available_frames()
    }
}

impl Runtime {
    #[cfg(feature = "mic")]
    pub(super) async fn publish_audio(
        self,
        samples: &[i16; SAMPLES_PER_BLOCK],
        peak_left: u16,
        peak_right: u16,
    ) -> u32 {
        let mut queue = self.service.mic_queue.lock().await;
        queue.push(samples, peak_left, peak_right)
    }

    #[cfg(feature = "speaker")]
    pub(super) async fn read_speaker_interleaved(self, out: &mut [i16]) -> usize {
        let mut queue = self.service.speaker_queue.lock().await;
        queue.read_interleaved(out)
    }
}
