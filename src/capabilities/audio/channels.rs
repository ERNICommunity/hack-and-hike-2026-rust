//! Cross-core synchronization for the private shared audio runtime.

#[cfg(feature = "speaker-synth")]
use core::sync::atomic::{AtomicU32, Ordering};

#[cfg(any(feature = "mic", feature = "speaker-synth"))]
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
#[cfg(feature = "mic")]
use embassy_sync::mutex::Mutex;
#[cfg(feature = "speaker-synth")]
use embassy_sync::signal::Signal;
use static_cell::StaticCell;

#[cfg(feature = "mic")]
use crate::capabilities::mic::{MicBlockInfo, QUEUE_CAPACITY_BLOCKS, SAMPLES_PER_BLOCK};

#[cfg(feature = "speaker-synth")]
use super::PlaybackSettings;

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

    fn push(
        &mut self,
        samples: &[i16; SAMPLES_PER_BLOCK],
        peak_left: u16,
        peak_right: u16,
    ) -> u32 {
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

#[cfg(feature = "mic")]
type MicQueueStore = Mutex<CriticalSectionRawMutex, MicQueue>;
#[cfg(feature = "speaker-synth")]
type PlaybackSignal = Signal<CriticalSectionRawMutex, PlaybackSettings>;

struct Service {
    #[cfg(feature = "mic")]
    mic_queue: MicQueueStore,
    #[cfg(feature = "speaker-synth")]
    playback_settings: PlaybackSignal,
    #[cfg(feature = "speaker-synth")]
    one_shot_sequence: AtomicU32,
}

impl Service {
    const fn new() -> Self {
        Self {
            #[cfg(feature = "mic")]
            mic_queue: Mutex::new(MicQueue::new()),
            #[cfg(feature = "speaker-synth")]
            playback_settings: Signal::new(),
            #[cfg(feature = "speaker-synth")]
            one_shot_sequence: AtomicU32::new(0),
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

/// CPU0 command endpoint used by the stock speaker synth application.
#[cfg(feature = "speaker-synth")]
pub(crate) struct PlaybackControl {
    service: &'static Service,
}

pub(crate) struct Endpoints {
    pub(crate) runtime: Runtime,
    #[cfg(feature = "mic")]
    pub(crate) mic: MicReader,
    #[cfg(feature = "speaker-synth")]
    pub(crate) playback: PlaybackControl,
}

pub(crate) fn init_endpoints() -> Endpoints {
    let service: &'static Service = SERVICE.init(Service::new());
    Endpoints {
        runtime: Runtime { service },
        #[cfg(feature = "mic")]
        mic: MicReader { service },
        #[cfg(feature = "speaker-synth")]
        playback: PlaybackControl { service },
    }
}

#[cfg(feature = "mic")]
impl MicReader {
    pub(crate) fn try_read(
        &mut self,
        out: &mut [i16; SAMPLES_PER_BLOCK],
    ) -> Option<MicBlockInfo> {
        let mut queue = self.service.mic_queue.try_lock().ok()?;
        queue.pop(out)
    }
}

#[cfg(feature = "speaker-synth")]
impl PlaybackControl {
    pub(crate) fn set(&mut self, settings: PlaybackSettings) {
        self.service.playback_settings.signal(settings);
    }

    pub(crate) fn play_one_shot(&mut self) {
        self.service
            .one_shot_sequence
            .fetch_add(1, Ordering::Release);
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

    #[cfg(feature = "speaker-synth")]
    pub(super) fn take_playback_settings(self) -> Option<PlaybackSettings> {
        self.service.playback_settings.try_take()
    }

    #[cfg(feature = "speaker-synth")]
    pub(super) fn one_shot_sequence(self) -> u32 {
        self.service.one_shot_sequence.load(Ordering::Acquire)
    }
}
