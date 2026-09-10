//! CPU1 speaker TX DMA and playback synthesis.

use esp_hal::{Async, i2s::master::I2sTx};

use crate::support::diagnostics;

use super::{PlaybackSettings, channels::Runtime, chime::FlashChime, melody::MelodySynth};

// Render speaker data into a small staging block and feed it through `push`,
// whose esp-hal 1.1.x implementation correctly propagates TX-underrun errors.
// 1024 bytes is 16 ms of stereo 16-bit audio at 16 kHz, keeping controls snappy.
const PLAYBACK_FILL_BYTES: usize = 1_024;
const _: () = assert!(PLAYBACK_FILL_BYTES % 4 == 0);

// The one-shot is derived offline from the user-supplied MP3 and stored as
// flash-resident IMA ADPCM; decoding is incremental and allocation-free.

#[embassy_executor::task]
pub(super) async fn playback_task(
    i2s_tx: I2sTx<'static, Async>,
    tx_buffer: &'static mut [u8],
    runtime: Runtime,
) {
    // Circular TX starts reading immediately. Silence the complete ring first
    // so startup can never replay uninitialized/stale bytes before the task's
    // first refill.
    tx_buffer.fill(0);

    let mut transfer = i2s_tx
        .write_dma_circular_async(tx_buffer)
        .expect("Failed to start circular I2S TX DMA");
    let mut engine = PlaybackEngine::new();
    let mut settings = PlaybackSettings::DEFAULT;
    let mut one_shot_seen = runtime.one_shot_sequence();
    let mut staging = [0u8; PLAYBACK_FILL_BYTES];
    let mut staging_offset = staging.len();

    loop {
        if staging_offset == staging.len() {
            if let Some(next) = runtime.take_playback_settings() {
                if next.melody_playing && !settings.melody_playing {
                    engine.melody.restart();
                }
                settings = next;
            }

            let one_shot_sequence = runtime.one_shot_sequence();
            if one_shot_sequence != one_shot_seen {
                one_shot_seen = one_shot_sequence;
                engine.chime.restart();
            }

            engine.fill(&mut staging, settings);
            staging_offset = 0;
        }

        match transfer.push(&staging[staging_offset..]).await {
            Ok(written) if written != 0 => staging_offset += written,
            Ok(_) => {}
            Err(_) => {
                // `push` propagates esp-hal's DmaError::Late, unlike `push_with`
                // in the pinned 1.1.x HAL. A late ring cannot be repaired through
                // this API because the transfer owns I2sTx, so fail closed with a
                // controlled reboot instead of replaying stale samples forever.
                diagnostics::record_audio_playback_error();
                ::log::error!("I2S TX DMA underrun; rebooting to recover audio");
                esp_hal::system::software_reset();
            }
        }
    }
}

struct PlaybackEngine {
    melody: MelodySynth,
    chime: FlashChime,
    pending_frame: [u8; 4],
    pending_offset: usize,
}

impl PlaybackEngine {
    const fn new() -> Self {
        Self {
            melody: MelodySynth::new(),
            chime: FlashChime::new(),
            pending_frame: [0; 4],
            pending_offset: 4,
        }
    }

    /// Fill every byte handed to the playback staging buffer.
    ///
    /// Keeping a partially emitted stereo frame makes this helper byte-safe and
    /// independent of the current staging size.
    fn fill(&mut self, bytes: &mut [u8], settings: PlaybackSettings) -> usize {
        for byte in bytes.iter_mut() {
            if self.pending_offset == self.pending_frame.len() {
                self.prepare_frame(settings);
                self.pending_offset = 0;
            }

            *byte = self.pending_frame[self.pending_offset];
            self.pending_offset += 1;
        }

        bytes.len()
    }

    fn prepare_frame(&mut self, settings: PlaybackSettings) {
        let melody = if settings.melody_playing {
            self.melody.next_sample(settings.tempo, settings.pitch)
        } else {
            0
        };
        let sample = saturating_mix(melody, self.chime.next_sample());
        let encoded = sample.to_le_bytes();
        self.pending_frame = [encoded[0], encoded[1], encoded[0], encoded[1]];
    }
}

fn saturating_mix(a: i16, b: i16) -> i16 {
    i32::from(a)
        .saturating_add(i32::from(b))
        .clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16
}
