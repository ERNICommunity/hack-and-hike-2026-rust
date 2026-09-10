//! Cross-core audio input and playback command synchronization.

use core::sync::atomic::{AtomicU32, Ordering};

use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex,
    mutex::Mutex,
    signal::Signal,
};
use static_cell::StaticCell;

use super::{AudioBlockInfo, BLOCK_SAMPLES, PlaybackSettings};

struct LatestAudio {
    samples: [i16; BLOCK_SAMPLES],
    info: AudioBlockInfo,
}

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

type LatestAudioStore = Mutex<CriticalSectionRawMutex, LatestAudio>;
type PlaybackSignal = Signal<CriticalSectionRawMutex, PlaybackSettings>;

struct Service {
    latest_audio: LatestAudioStore,
    playback_settings: PlaybackSignal,
    one_shot_sequence: AtomicU32,
}

impl Service {
    const fn new() -> Self {
        Self {
            latest_audio: Mutex::new(LatestAudio::new()),
            playback_settings: Signal::new(),
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
pub struct Input {
    service: &'static Service,
}

/// CPU0 command endpoint for the CPU1 audio-output service.
pub struct PlaybackControl {
    service: &'static Service,
}

pub(crate) struct Endpoints {
    pub(crate) runtime: Runtime,
    pub(crate) input: Input,
    pub(crate) playback: PlaybackControl,
}

pub(crate) fn init_endpoints() -> Endpoints {
    let service: &'static Service = SERVICE.init(Service::new());
    Endpoints {
        runtime: Runtime { service },
        input: Input { service },
        playback: PlaybackControl { service },
    }
}

impl Input {
    pub fn copy_latest_interleaved(
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

impl PlaybackControl {
    pub fn set(&mut self, settings: PlaybackSettings) {
        self.service.playback_settings.signal(settings);
    }

    pub fn play_one_shot(&mut self) {
        self.service
            .one_shot_sequence
            .fetch_add(1, Ordering::Release);
    }
}

impl Runtime {
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

    pub(super) fn take_playback_settings(self) -> Option<PlaybackSettings> {
        self.service.playback_settings.try_take()
    }

    pub(super) fn one_shot_sequence(self) -> u32 {
        self.service.one_shot_sequence.load(Ordering::Acquire)
    }
}
