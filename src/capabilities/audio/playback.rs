//! CPU1 owner of I2S TX: drains the speaker queue into DMA and plays silence
//! whenever the application has nothing queued.

use esp_hal::{Async, dma::DmaTxStreamBuf, i2s::master::I2sTx};
use log::warn;

use super::channels::Runtime;

/// Bytes handed to DMA per step: 16 ms of stereo 16-bit audio at 16 kHz.
const FILL_BYTES: usize = 1_024;
const FILL_SAMPLES: usize = FILL_BYTES / core::mem::size_of::<i16>();

/// Play forever. A DMA error restarts the transfer instead of ending audio.
#[embassy_executor::task]
pub(super) async fn playback_task(
    mut i2s_tx: I2sTx<'static, Async>,
    mut tx_buffer: DmaTxStreamBuf,
    runtime: Runtime,
) {
    let mut staging = [0u8; FILL_BYTES];
    let mut pcm = [0i16; FILL_SAMPLES];

    loop {
        // Start from a full buffer of silence so the amplifier never sees
        // stale data.
        tx_buffer.push_with(|buffer| {
            buffer.fill(0);
            buffer.len()
        });
        let mut transfer = i2s_tx
            .write(tx_buffer)
            .map_err(|(error, _, _)| error)
            .expect("I2S TX DMA start failed");
        let mut staged = staging.len();

        let error = loop {
            // A descriptor EOF frees about 4 KiB at once; refill everything
            // that is writable before waiting for the next EOF.
            if transfer.available_bytes() == 0 {
                if let Err(error) = transfer.wait_for_available_async().await {
                    break error;
                }
                continue;
            }

            if staged == staging.len() {
                staging.fill(0);
                let frames = runtime.read_speaker(&mut pcm);
                for (bytes, sample) in staging.chunks_exact_mut(2).zip(&pcm[..frames * 2]) {
                    bytes.copy_from_slice(&sample.to_le_bytes());
                }
                staged = 0;
            }

            staged += transfer.push(&staging[staged..]);
        };

        warn!("Speaker DMA error {:?}; restarting playback", error);
        (i2s_tx, tx_buffer) = transfer.stop();
    }
}
