//! Microphone and speaker.
//!
//! Applications get two independent handles, [`Microphone`] and [`Speaker`].
//! Both use signed 16-bit samples at [`SAMPLE_RATE_HZ`], with two channels
//! interleaved: left, right, left, right, ... One left sample and one right
//! sample together are a *frame*.
//!
//! Inside, both use one I2S peripheral on CPU1. I2S is a bus for digital
//! audio. The speaker side generates its clock, and the microphone side uses
//! the same clock. Two codec chips convert the signals: the ES7210 digitizes
//! the two microphones, and the AW88298 amplifies the speaker signal.
//!
//! The board has only one loudspeaker. The applications in this project
//! write the same sample to both channels. This is the safe choice.

mod codecs;
mod runtime;

use core::cell::RefCell;

use embassy_sync::{
    blocking_mutex::{Mutex, raw::CriticalSectionRawMutex},
    channel::Channel,
};
use esp_hal::peripherals::{DMA_CH0, GPIO0, GPIO13, GPIO14, GPIO33, GPIO34, I2S0};
use log::debug;

use hack_and_hike_core::audio::FrameRing;

pub(crate) use codecs::init_codecs;
pub(crate) use runtime::spawn;

/// Sample rate of both the microphone and the speaker.
pub const SAMPLE_RATE_HZ: u32 = 16_000;
/// Samples per frame: left and right.
pub const CHANNELS: usize = 2;
/// Stereo frames in one microphone block (32 ms at 16 kHz).
pub const FRAMES_PER_BLOCK: usize = 512;
/// Samples in one microphone block: `FRAMES_PER_BLOCK` frames of `CHANNELS`.
pub const SAMPLES_PER_BLOCK: usize = FRAMES_PER_BLOCK * CHANNELS;

/// Size of the microphone queue in blocks. When the queue is full, the
/// oldest unread block is dropped. Eight blocks are about 256 ms. That is
/// enough for a loop iteration that waits on a slow capability. The camera's
/// wait for a frame is the slowest case. Each block is 2 KiB.
const MIC_QUEUE_BLOCKS: usize = 8;
/// Size of the speaker queue in stereo frames: 1,024 frames, about 64 ms.
///
/// When the DMA has sent one descriptor, the playback task refills about
/// 1,023 frames at once. A DMA descriptor is one block of the DMA buffer.
/// The queue is at least that large. So a queue that the application keeps
/// full can supply a whole refill.
const SPEAKER_QUEUE_FRAMES: usize = 1_024;

/// The speaker ring: room for [`SPEAKER_QUEUE_FRAMES`] frames, stored as
/// interleaved samples.
type SpeakerQueue = FrameRing<{ SPEAKER_QUEUE_FRAMES * CHANNELS }, CHANNELS>;

/// The I2S peripheral and the pins connected to the codecs. The audio task
/// on CPU1 takes them.
pub(crate) struct Resources {
    /// The I2S controller.
    pub(crate) i2s0: I2S0<'static>,
    /// The DMA channel that moves samples in both directions.
    pub(crate) dma: DMA_CH0<'static>,
    /// Master clock (MCLK) for the codecs.
    pub(crate) mclk: GPIO0<'static>,
    /// Bit clock (BCLK, also called BCK): one cycle for each data bit.
    pub(crate) bclk: GPIO34<'static>,
    /// Word select (WS, also called LRCK): shows whether the current sample
    /// is left or right.
    pub(crate) word_select: GPIO33<'static>,
    /// Samples from the microphone codec (ES7210).
    pub(crate) data_in: GPIO14<'static>,
    /// Samples to the speaker amplifier (AW88298).
    pub(crate) data_out: GPIO13<'static>,
}

/// Information that comes with every microphone block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MicBlockInfo {
    /// Increases by one with every captured block. A gap in the numbers
    /// means that blocks were lost.
    pub sequence: u32,
    /// Number of blocks dropped since start, because the application did
    /// not read them in time.
    pub dropped_blocks: u32,
    /// Largest absolute value of the left samples in this block, 0 to 32768.
    pub peak_left: u16,
    /// Largest absolute value of the right samples in this block, 0 to
    /// 32768.
    pub peak_right: u16,
}

/// One captured microphone block, on its way from CPU1 to CPU0.
#[derive(Clone, Copy)]
struct MicBlock {
    /// Interleaved stereo samples.
    samples: [i16; SAMPLES_PER_BLOCK],
    /// Sequence number, drop count and peaks.
    info: MicBlockInfo,
}

impl MicBlock {
    /// A block with all values 0. The capture task starts from it.
    const SILENT: Self = Self {
        samples: [0; SAMPLES_PER_BLOCK],
        info: MicBlockInfo {
            sequence: 0,
            dropped_blocks: 0,
            peak_left: 0,
            peak_right: 0,
        },
    };
}

/// The queues that the handles (CPU0) and the audio tasks (CPU1) share.
struct Service {
    /// Captured blocks that wait for the application.
    mic_blocks: Channel<CriticalSectionRawMutex, MicBlock, MIC_QUEUE_BLOCKS>,
    /// Samples that wait to be played. The mutex uses a critical section,
    /// which blocks interrupts and the other core for a short time. This is
    /// acceptable, because both cores use the ring and each access is only
    /// a short copy.
    speaker: Mutex<CriticalSectionRawMutex, RefCell<SpeakerQueue>>,
}

/// The only set of audio queues. A plain `static` works on both cores,
/// because every field does its own synchronization.
static SERVICE: Service = Service {
    mic_blocks: Channel::new(),
    speaker: Mutex::new(RefCell::new(SpeakerQueue::new())),
};

/// Application handle for the microphones.
///
/// Audio comes in blocks of [`FRAMES_PER_BLOCK`] stereo frames, 32 ms each.
/// Up to eight blocks (about 256 ms) wait in a queue. When the application
/// does not read them in time, the oldest block is dropped, and
/// [`dropped_blocks`](MicBlockInfo::dropped_blocks) counts it.
///
/// ```ignore
/// let mut block = [0i16; audio::SAMPLES_PER_BLOCK];
/// while let Some(info) = microphone.next_block(&mut block) {
///     let loud = info.peak_left > 10_000;
/// }
/// ```
///
/// A block is 2 KiB. Keep the buffer in a struct or a `static`, not on a
/// small task stack.
pub struct Microphone {
    /// Points at the queues shared with the CPU1 audio tasks.
    service: &'static Service,
}

impl Microphone {
    /// Copy the oldest unread block into `samples` and return its
    /// information. Return `None` when no complete block is waiting. Never
    /// waits.
    pub fn next_block(&mut self, samples: &mut [i16; SAMPLES_PER_BLOCK]) -> Option<MicBlockInfo> {
        let block = self.service.mic_blocks.try_receive().ok()?;
        samples.copy_from_slice(&block.samples);
        Some(block.info)
    }
}

/// Application handle for the speaker.
///
/// The application puts interleaved stereo samples into a queue. CPU1 takes
/// them from the queue and sends them to the amplifier. When the queue is
/// empty, the speaker plays silence. The queue holds 1024 frames (64 ms). So
/// add a few samples on every loop iteration. Do not try to write a whole
/// sound at once.
///
/// ```ignore
/// let mut chunk = [0i16; 128 * audio::CHANNELS];
/// let frames = speaker.available_frames().min(128);
/// for frame in chunk[..frames * audio::CHANNELS].chunks_exact_mut(audio::CHANNELS) {
///     frame.fill(tone.next_sample(0.2));
/// }
/// speaker.write(&chunk[..frames * audio::CHANNELS]);
/// ```
pub struct Speaker {
    /// Points at the queues shared with the CPU1 audio tasks.
    service: &'static Service,
}

impl Speaker {
    /// The number of stereo frames that fit into the queue now.
    ///
    /// Only the application adds to the queue, so the free space can only
    /// grow until the next [`write`]. A [`write`] of at most this many frames
    /// is always accepted completely.
    ///
    /// [`write`]: Speaker::write
    pub fn available_frames(&self) -> usize {
        self.service
            .speaker
            .lock(|queue| queue.borrow().free_frames())
    }

    /// Add interleaved stereo samples (left, right, left, right, ...) to the
    /// queue. Never waits.
    ///
    /// Return the number of frames accepted. When the queue does not have
    /// room for all frames, only the first frames that fit are accepted. A
    /// last sample without its partner is ignored.
    pub fn write(&mut self, samples: &[i16]) -> usize {
        self.service
            .speaker
            .lock(|queue| queue.borrow_mut().write(samples))
    }
}

/// The CPU1 side of the queues.
#[derive(Clone, Copy)]
pub(crate) struct Runtime {
    /// Points at the queues shared with the application's handles on CPU0.
    service: &'static Service,
}

impl Runtime {
    /// Whether the microphone queue is full. If it is, the next
    /// [`Runtime::publish_microphone_block`] drops the oldest unread block.
    fn microphone_queue_is_full(self) -> bool {
        self.service.mic_blocks.is_full()
    }

    /// Put a captured block into the queue for the application. When the
    /// queue is full, drop the oldest unread block first.
    fn publish_microphone_block(self, block: MicBlock) {
        if self.service.mic_blocks.is_full() {
            let _dropped = self.service.mic_blocks.try_receive();
        }
        if self.service.mic_blocks.try_send(block).is_err() {
            debug!("Microphone block lost: queue still full");
        }
    }

    /// Move whole stereo frames from the speaker queue into `out`, for
    /// playback. Return the number of frames, not samples.
    fn read_speaker(self, out: &mut [i16]) -> usize {
        self.service
            .speaker
            .lock(|queue| queue.borrow_mut().read(out))
    }
}

/// The ends of the audio queues. The board creates them once.
pub(crate) struct Endpoints {
    /// The microphone handle, for the application.
    pub(crate) microphone: Microphone,
    /// The speaker handle, for the application.
    pub(crate) speaker: Speaker,
    /// The CPU1 side, for the audio tasks.
    pub(crate) runtime: Runtime,
}

/// All ends of the audio queues. [`Board::init`](crate::Board::init) calls
/// it once. Every call returns ends of the same queues. A second
/// [`Speaker`] would break the promise of [`Speaker::available_frames`],
/// which expects only one writer.
pub(crate) fn endpoints() -> Endpoints {
    Endpoints {
        microphone: Microphone { service: &SERVICE },
        speaker: Speaker { service: &SERVICE },
        runtime: Runtime { service: &SERVICE },
    }
}
