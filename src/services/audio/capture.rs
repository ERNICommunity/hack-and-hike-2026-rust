//! CPU1 I2S setup and continuous microphone RX DMA acquisition.

use embassy_executor::Spawner;
use esp_hal::{
    gpio::NoPin,
    i2s::master::{Channels, Config as I2sConfig, DataFormat, I2s},
    time::Rate,
};

use crate::support::{diagnostics, memory::storage};

use super::{
    BLOCK_FRAMES, BLOCK_SAMPLES, CHANNELS, Resources, SAMPLE_RATE_HZ, channels::Runtime, playback,
};

const RX_DMA_BUFFER_BYTES: usize = 32 * 1024;
// esp-hal 1.1.x circular RX requires each `pop` destination to hold all bytes
// currently available in the DMA ring. Drain into a full-ring PSRAM snapshot,
// then process that snapshot one presentation block at a time so CPU1 still
// yields regularly to the speaker producer while microphone RX catches up.
const RX_PROCESS_CHUNK_BYTES: usize = BLOCK_FRAMES * CHANNELS * 2;
// esp-hal 1.1.x uses 4092-byte DMA chunks and balances circular buffers that
// fit within two chunks across three descriptors. 8192 misses that path by only
// 8 bytes, producing a pathological 4092 + 4092 + 8-byte TX ring.
const TX_DMA_BUFFER_BYTES: usize = 8_184;
const _: () = assert!(RX_PROCESS_CHUNK_BYTES % 4 == 0);
const _: () = assert!(TX_DMA_BUFFER_BYTES % 4 == 0);

/// Yield exactly once even if the caller has more buffered work available.
///
/// Embassy is cooperative: an `.await` that is immediately ready does not give
/// sibling tasks a turn. This helper forces the microphone drain loop to return
/// `Pending` once so the continuous speaker refill can run before RX catch-up
/// continues.
async fn yield_to_executor() {
    use core::task::Poll;

    let mut yielded = false;
    core::future::poll_fn(|cx| {
        if yielded {
            Poll::Ready(())
        } else {
            yielded = true;
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    })
    .await;
}

/// Own I2S0 on CPU1, start the continuous speaker writer, then drain microphone
/// RX forever. TX is the physical BCLK/WS master; RX follows the same signals
/// through the peripheral's internal signal-loopback path.
#[embassy_executor::task]
pub(crate) async fn capture_task(resources: Resources, spawner: Spawner, runtime: Runtime) {
    let Resources {
        i2s0,
        dma,
        mclk,
        bclk,
        word_select,
        data_in,
        data_out,
    } = resources;

    let (rx_buffer, rx_descriptors, tx_buffer, tx_descriptors) =
        esp_hal::dma_circular_buffers!(RX_DMA_BUFFER_BYTES, TX_DMA_BUFFER_BYTES);

    let i2s = I2s::new(
        i2s0,
        dma,
        I2sConfig::new_tdm_philips()
            .with_signal_loopback(true)
            .with_sample_rate(Rate::from_hz(SAMPLE_RATE_HZ))
            .with_data_format(DataFormat::Data16Channel16)
            .with_channels(Channels::STEREO),
    )
    .expect("Failed to configure full-duplex I2S0")
    .with_mclk(mclk)
    .into_async();

    let i2s_tx = i2s
        .i2s_tx
        .with_bclk(bclk)
        .with_ws(word_select)
        .with_dout(data_out)
        .build(tx_descriptors);
    let i2s_rx = i2s
        .i2s_rx
        .with_bclk(NoPin)
        .with_ws(NoPin)
        .with_din(data_in)
        .build(rx_descriptors);

    spawner.spawn(
        playback::playback_task(i2s_tx, tx_buffer, runtime)
            .expect("Failed to allocate speaker playback task"),
    );

    let mut transfer = i2s_rx
        .read_dma_circular_async(rx_buffer)
        .expect("Failed to start circular I2S RX DMA");

    ::log::info!(
        "Audio DMA started: {} Hz, stereo, 16-bit, mic + speaker full duplex",
        SAMPLE_RATE_HZ
    );

    let mut dma_drain = storage::FixedPsramBuffer::filled(RX_DMA_BUFFER_BYTES, 0u8);
    let mut samples = [0i16; BLOCK_SAMPLES];
    let mut frame_index = 0usize;
    let mut peak_left = 0u16;
    let mut peak_right = 0u16;
    let mut first_block = true;

    loop {
        let count = match transfer.pop(dma_drain.as_mut_slice()).await {
            Ok(count) => count,
            Err(_) => {
                diagnostics::record_audio_capture_error();
                panic!("I2S circular DMA read failed");
            }
        };

        if count == RX_DMA_BUFFER_BYTES {
            diagnostics::record_audio_full_drain();
        }

        for processing_chunk in dma_drain.as_slice()[..count].chunks(RX_PROCESS_CHUNK_BYTES) {
            for frame_bytes in processing_chunk.chunks_exact(4) {
                let left = i16::from_le_bytes([frame_bytes[0], frame_bytes[1]]);
                let right = i16::from_le_bytes([frame_bytes[2], frame_bytes[3]]);
                let sample_index = frame_index * CHANNELS;
                samples[sample_index] = left;
                samples[sample_index + 1] = right;
                peak_left = peak_left.max(left.unsigned_abs());
                peak_right = peak_right.max(right.unsigned_abs());
                frame_index += 1;

                if frame_index == BLOCK_FRAMES {
                    let sequence = runtime.publish_audio(&samples, peak_left, peak_right).await;
                    if first_block {
                        first_block = false;
                        ::log::info!(
                            "First audio block captured: seq={}, peak L={}, R={}",
                            sequence,
                            peak_left,
                            peak_right
                        );
                    }
                    frame_index = 0;
                    peak_left = 0;
                    peak_right = 0;
                }
            }

            yield_to_executor().await;
        }
    }
}
