//! Microphone and speaker.
//!
//! Applications see two independent handles, [`Microphone`] and [`Speaker`],
//! that exchange audio as signed 16-bit samples at [`SAMPLE_RATE_HZ`], two
//! channels interleaved: left, right, left, right, ... One left and right
//! sample together are a *frame*.
//!
//! Underneath, both share one I2S peripheral and clock on CPU1: the speaker
//! path generates the clock and the microphone path follows it. Two codec
//! chips convert: the ES7210 digitizes the two microphones, the AW88298
//! amplifies the speaker.
//!
//! The board has a single loudspeaker. The applications here write the same
//! sample to both channels, which is the safe choice.

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

/// Microphone blocks the application may fall behind by before the oldest
/// unread block is dropped.
const MIC_QUEUE_BLOCKS: usize = 4;
/// Speaker frames buffered ahead of playback. One DMA descriptor refill takes
/// up to 1023 frames, so the queue holds at least that much to keep a refill
/// from draining it before the application tops it up.
const SPEAKER_QUEUE_FRAMES: usize = 1_024;

type SpeakerQueue = FrameRing<{ SPEAKER_QUEUE_FRAMES * CHANNELS }, CHANNELS>;

/// The I2S peripheral and pins wired to the codecs, owned by CPU1.
pub(crate) struct Resources {
    /// The I2S controller.
    pub(crate) i2s0: I2S0<'static>,
    /// The DMA channel streaming samples in both directions.
    pub(crate) dma: DMA_CH0<'static>,
    /// Master clock for the codecs.
    pub(crate) mclk: GPIO0<'static>,
    /// Bit clock.
    pub(crate) bclk: GPIO34<'static>,
    /// Word select: which channel the current sample belongs to.
    pub(crate) word_select: GPIO33<'static>,
    /// Samples from the microphone codec.
    pub(crate) data_in: GPIO14<'static>,
    /// Samples to the speaker amplifier.
    pub(crate) data_out: GPIO13<'static>,
}

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

/// One captured microphone block as it travels from CPU1 to CPU0.
#[derive(Clone, Copy)]
struct MicBlock {
    /// Interleaved stereo samples.
    samples: [i16; SAMPLES_PER_BLOCK],
    /// Sequence number, drop count and peaks.
    info: MicBlockInfo,
}

impl MicBlock {
    /// An all-zero block, to start building from.
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

/// The queues shared by the handles (CPU0) and the audio tasks (CPU1).
struct Service {
    /// Captured blocks waiting for the application.
    mic_blocks: Channel<CriticalSectionRawMutex, MicBlock, MIC_QUEUE_BLOCKS>,
    /// Samples waiting to be played. A critical-section mutex, because both
    /// cores touch the ring and each access is a short copy.
    speaker: Mutex<CriticalSectionRawMutex, RefCell<SpeakerQueue>>,
}

static SERVICE: Service = Service {
    mic_blocks: Channel::new(),
    speaker: Mutex::new(RefCell::new(SpeakerQueue::new())),
};

/// Application handle for the microphones.
///
/// Audio arrives in blocks of [`FRAMES_PER_BLOCK`] stereo frames, 32 ms each.
/// Blocks wait in a small queue; when the application does not read them in
/// time the oldest block is dropped and
/// [`dropped_blocks`](MicBlockInfo::dropped_blocks) counts it.
///
/// ```ignore
/// let mut block = [0i16; audio::SAMPLES_PER_BLOCK];
/// while let Some(info) = microphone.next_block(&mut block) {
///     let loud = info.peak_left > 10_000;
/// }
/// ```
///
/// A block is 2 KiB; keep the buffer in a struct or a `static`, not on a
/// small task stack.
pub struct Microphone {
    service: &'static Service,
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

/// Application handle for the speaker.
///
/// The application queues interleaved stereo samples; CPU1 drains the queue
/// into the amplifier and plays silence whenever the queue runs empty. The
/// queue holds about 64 ms, so feed it a little on every loop iteration
/// instead of writing a whole sound at once.
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
    service: &'static Service,
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

/// CPU1 side of the queues.
#[derive(Clone, Copy)]
pub(crate) struct Runtime {
    service: &'static Service,
}

impl Runtime {
    /// True when publishing another block would drop the oldest unread one.
    fn microphone_queue_is_full(self) -> bool {
        self.service.mic_blocks.is_full()
    }

    /// Queue a captured block for the application, dropping the oldest unread
    /// block first when the application has fallen behind.
    fn publish_microphone_block(self, block: MicBlock) {
        if self.service.mic_blocks.is_full() {
            let _dropped = self.service.mic_blocks.try_receive();
        }
        if self.service.mic_blocks.try_send(block).is_err() {
            debug!("Microphone block lost: queue still full");
        }
    }

    /// Take queued speaker frames for playback. Returns how many frames were read.
    fn read_speaker(self, out: &mut [i16]) -> usize {
        self.service
            .speaker
            .lock(|queue| queue.borrow_mut().read(out))
    }
}

/// The ends of the audio queues, created once by the board.
pub(crate) struct Endpoints {
    /// For the application.
    pub(crate) microphone: Microphone,
    /// For the application.
    pub(crate) speaker: Speaker,
    /// For the CPU1 audio tasks.
    pub(crate) runtime: Runtime,
}

/// All ends of the audio queues.
pub(crate) fn endpoints() -> Endpoints {
    Endpoints {
        microphone: Microphone { service: &SERVICE },
        speaker: Speaker { service: &SERVICE },
        runtime: Runtime { service: &SERVICE },
    }
}
