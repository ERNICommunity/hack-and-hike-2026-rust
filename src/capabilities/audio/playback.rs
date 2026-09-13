//! CPU1 TX DMA owner for the shared audio clock domain.
//!
//! Speaker applications enqueue raw PCM through the public capability. This task
//! only drains that bounded queue into I2S TX and substitutes silence whenever
//! no complete speaker frames are available. Mic-only builds use the same TX
//! owner to provide the physical BCLK/WS clock domain without owning a speaker.

use esp_hal::{Async, dma::DmaTxStreamBuf, i2s::master::I2sTx};

use crate::support::diagnostics;

use super::channels::Runtime;

// 1024 bytes is 16 ms of stereo 16-bit audio at 16 kHz.
const PLAYBACK_FILL_BYTES: usize = 1_024;
#[cfg(feature = "speaker")]
const PLAYBACK_FILL_SAMPLES: usize = PLAYBACK_FILL_BYTES / core::mem::size_of::<i16>();
const _: () = assert!(PLAYBACK_FILL_BYTES.is_multiple_of(4));

#[embassy_executor::task]
pub(super) async fn playback_task(
    i2s_tx: I2sTx<'static, Async>,
    mut tx_buffer: DmaTxStreamBuf,
    runtime: Runtime,
) {
    tx_buffer.push_with(|buffer| {
        buffer.fill(0);
        buffer.len()
    });

    let mut transfer = i2s_tx
        .write(tx_buffer)
        .map_err(|(error, _, _)| error)
        .expect("Failed to start circular I2S TX DMA");
    let mut staging = [0u8; PLAYBACK_FILL_BYTES];
    let mut staging_offset = staging.len();
    #[cfg(feature = "speaker")]
    let mut pcm = [0i16; PLAYBACK_FILL_SAMPLES];

    loop {
        // A TX descriptor EOF frees roughly one descriptor (4092 bytes). Refill
        // all currently writable stream capacity before waiting for another EOF;
        // otherwise a 1024-byte staging chunk would replenish only one quarter
        // of what DMA consumed and the descriptor ring would inevitably drain to
        // TotalEof.
        if transfer.available_bytes() == 0 {
            if transfer.wait_for_available_async().await.is_err() {
                handle_dma_underrun();
            }
            continue;
        }

        if staging_offset == staging.len() {
            staging.fill(0);

            #[cfg(feature = "speaker")]
            {
                let frames = runtime.read_speaker_interleaved(&mut pcm).await;
                let sample_count = frames * 2;
                for (encoded, sample) in staging
                    .chunks_exact_mut(2)
                    .zip(pcm[..sample_count].iter().copied())
                {
                    encoded.copy_from_slice(&sample.to_le_bytes());
                }
            }
            #[cfg(not(feature = "speaker"))]
            let _ = runtime;

            staging_offset = 0;
        }

        let written = transfer.push(&staging[staging_offset..]);
        if written != 0 {
            staging_offset += written;
        }
    }
}

fn handle_dma_underrun() -> ! {
    diagnostics::record_audio_playback_error();
    ::log::error!("I2S TX DMA underrun; rebooting to recover audio");
    esp_hal::system::software_reset();
}
