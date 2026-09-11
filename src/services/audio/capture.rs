//! CPU1 shared I2S setup and optional microphone RX acquisition.

use embassy_executor::Spawner;
use esp_hal::{
    gpio::NoPin,
    i2s::master::{Channels, Config as I2sConfig, DataFormat, I2s},
    time::Rate,
};

#[cfg(feature = "mic")]
use crate::support::{diagnostics, memory::storage};

use super::{Resources, SAMPLE_RATE_HZ, channels::Runtime, playback};
#[cfg(feature = "mic")]
use super::{BLOCK_FRAMES, BLOCK_SAMPLES, CHANNELS};

// The shared esp-hal buffer macro currently creates both descriptor sets. RX is
// only started and processed when the mic capability is enabled; speaker-only
// builds do not own the microphone GPIO or initialize the microphone codec.
const RX_DMA_BUFFER_BYTES: usize = 32 * 1024;
#[cfg(feature = "mic")]
const RX_PROCESS_CHUNK_BYTES: usize = BLOCK_FRAMES * CHANNELS * 2;
const TX_DMA_BUFFER_BYTES: usize = 8_184;
#[cfg(feature = "mic")]
const _: () = assert!(RX_PROCESS_CHUNK_BYTES % 4 == 0);
const _: () = assert!(TX_DMA_BUFFER_BYTES % 4 == 0);

#[cfg(feature = "mic")]
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

/// Own I2S0 on CPU1. TX always runs because it is the physical BCLK/WS master;
/// mic-only builds feed silence through TX while RX follows the same clocks.
#[embassy_executor::task]
pub(crate) async fn capture_task(resources: Resources, spawner: Spawner, runtime: Runtime) {
    let Resources {
        i2s0,
        dma,
        mclk,
        bclk,
        word_select,
        #[cfg(feature = "mic")]
        data_in,
        #[cfg(feature = "speaker")]
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
    .expect("Failed to configure shared audio I2S0")
    .with_mclk(mclk)
    .into_async();

    #[cfg(feature = "speaker")]
    let i2s_tx = i2s
        .i2s_tx
        .with_bclk(bclk)
        .with_ws(word_select)
        .with_dout(data_out)
        .build(tx_descriptors);
    #[cfg(not(feature = "speaker"))]
    let i2s_tx = i2s
        .i2s_tx
        .with_bclk(bclk)
        .with_ws(word_select)
        .with_dout(NoPin)
        .build(tx_descriptors);

    #[cfg(feature = "mic")]
    let i2s_rx = i2s
        .i2s_rx
        .with_bclk(NoPin)
        .with_ws(NoPin)
        .with_din(data_in)
        .build(rx_descriptors);

    spawner.spawn(
        playback::playback_task(i2s_tx, tx_buffer, runtime)
            .expect("Failed to allocate audio TX task"),
    );

    #[cfg(not(feature = "mic"))]
    {
        let _ = (rx_buffer, rx_descriptors);
        core::future::pending::<()>().await;
        return;
    }

    #[cfg(feature = "mic")]
    {
        let mut transfer = i2s_rx
            .read_dma_circular_async(rx_buffer)
            .expect("Failed to start circular I2S RX DMA");

        ::log::info!(
            "Microphone DMA started: {} Hz, stereo, 16-bit",
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
}
