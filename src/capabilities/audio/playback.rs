//! CPU1 TX DMA owner for the shared audio clock domain.
//!
//! Speaker applications enqueue raw PCM through the public capability. This task
//! only drains that bounded queue into I2S TX and substitutes silence whenever
//! no complete speaker frames are available. Mic-only builds use the same TX
//! owner to provide the physical BCLK/WS clock domain without owning a speaker.

use esp_hal::{Async, i2s::master::I2sTx};

use crate::support::diagnostics;

use super::channels::Runtime;

// 1024 bytes is 16 ms of stereo 16-bit audio at 16 kHz.
const PLAYBACK_FILL_BYTES: usize = 1_024;
#[cfg(feature = "speaker")]
const PLAYBACK_FILL_SAMPLES: usize = PLAYBACK_FILL_BYTES / core::mem::size_of::<i16>();
const _: () = assert!(PLAYBACK_FILL_BYTES % 4 == 0);

#[embassy_executor::task]
pub(super) async fn playback_task(
    i2s_tx: I2sTx<'static, Async>,
    tx_buffer: &'static mut [u8],
    runtime: Runtime,
) {
    tx_buffer.fill(0);

    let mut transfer = i2s_tx
        .write_dma_circular_async(tx_buffer)
        .expect("Failed to start circular I2S TX DMA");
    let mut staging = [0u8; PLAYBACK_FILL_BYTES];
    let mut staging_offset = staging.len();
    #[cfg(feature = "speaker")]
    let mut pcm = [0i16; PLAYBACK_FILL_SAMPLES];

    loop {
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

        match transfer.push(&staging[staging_offset..]).await {
            Ok(written) if written != 0 => staging_offset += written,
            Ok(_) => {}
            Err(_) => handle_dma_underrun(),
        }
    }
}

fn handle_dma_underrun() -> ! {
    diagnostics::record_audio_playback_error();
    ::log::error!("I2S TX DMA underrun; rebooting to recover audio");
    esp_hal::system::software_reset();
}
