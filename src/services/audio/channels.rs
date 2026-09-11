//! Cross-core audio synchronization for enabled audio capabilities.

#[cfg(feature = "speaker-synth")]
use core::sync::atomic::{AtomicU32, Ordering};

#[cfg(feature = "mic")]
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, mutex::Mutex};
#[cfg(feature = "speaker-synth")]
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use static_cell::StaticCell;

#[cfg(feature = "speaker-synth")]
use super::PlaybackSettings;
#[cfg(feature = "mic")]
use super::{AudioBlockInfo, BLOCK_SAMPLES};

#[cfg(feature = "mic")]
struct LatestAudio {
    samples: [i16; BLOCK_SAMPLES],
    info: AudioBlockInfo,
}

#[cfg(feature = "mic")]
impl LatestAudio {
    const fn new() -> Self {
        Self {
            samples: [0; BLOCK_SAMPLES],
            info: AudioBlockInfo {
                sequence: 0,
                peak_left: 0,
                peak_right: 0,
            },
        }
    }
}

#[cfg(feature = "mic")]
type LatestAudioStore = Mutex<CriticalSectionRawMutex, LatestAudio>;
#[cfg(feature = "speaker-synth")]
type PlaybackSignal = Signal<CriticalSectionRawMutex, PlaybackSettings>;

struct Service {
    #[cfg(feature = "mic")]
    latest_audio: LatestAudioStore,
    #[cfg(feature = "speaker-synth")]
    playback_settings: PlaybackSignal,
    #[cfg(feature = "speaker-synth")]
    one_shot_sequence: AtomicU32,
}

impl Service {
    const fn new() -> Self {
        Self {
            #[cfg(feature = "mic")]
            latest_audio: Mutex::new(LatestAudio::new()),
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

/// CPU0 input endpoint for the newest complete stereo microphone block.
#[cfg(feature = "mic")]
pub(crate) struct Input {
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
    pub(crate) input: Input,
    #[cfg(feature = "speaker-synth")]
    pub(crate) playback: PlaybackControl,
}

pub(crate) fn init_endpoints() -> Endpoints {
    let service: &'static Service = SERVICE.init(Service::new());
    Endpoints {
        runtime: Runtime { service },
        #[cfg(feature = "mic")]
        input: Input { service },
        #[cfg(feature = "speaker-synth")]
        playback: PlaybackControl { service },
    }
}

#[cfg(feature = "mic")]
impl Input {
    pub(crate) fn copy_latest_interleaved(
        &mut self,
        out: &mut [i16; BLOCK_SAMPLES],
    ) -> Option<AudioBlockInfo> {
        let latest = self.service.latest_audio.try_lock().ok()?;
        if latest.info.sequence == 0 {
            return None;
        }
        out.copy_from_slice(&latest.samples);
        Some(latest.info)
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
        samples: &[i16; BLOCK_SAMPLES],
        peak_left: u16,
        peak_right: u16,
    ) -> u32 {
        let mut latest = self.service.latest_audio.lock().await;
        latest.samples.copy_from_slice(samples);
        latest.info.sequence = latest.info.sequence.wrapping_add(1);
        latest.info.peak_left = peak_left;
        latest.info.peak_right = peak_right;
        latest.info.sequence
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
