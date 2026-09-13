//! CPU1 shared I2S setup and optional microphone RX acquisition.

use embassy_executor::Spawner;
use esp_hal::gpio::NoPin;
use esp_hal::{
    i2s::master::{Channels, DataFormat, I2s, TdmConfig},
    time::Rate,
};

use crate::{
    capabilities::mic::{CHANNELS, FRAMES_PER_BLOCK, SAMPLES_PER_BLOCK},
    support::memory::storage,
};

use super::{Resources, SAMPLE_RATE_HZ, channels::Runtime, playback};

const RX_DMA_BUFFER_BYTES: usize = 32 * 1024;
const RX_PROCESS_CHUNK_BYTES: usize = FRAMES_PER_BLOCK * CHANNELS * 2;
// esp-hal 1.2's streaming TX buffer currently requires at least four DMA
// descriptors. Keep the proven default 4092-byte descriptor geometry and
// deepen only the stream ring rather than shrinking descriptor chunks.
const TX_DMA_BUFFER_BYTES: usize = 4 * esp_hal::dma::CHUNK_SIZE;
const _: () = assert!(RX_PROCESS_CHUNK_BYTES.is_multiple_of(4));
const _: () = assert!(TX_DMA_BUFFER_BYTES.is_multiple_of(4));

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
        data_in,
        data_out,
    } = resources;

    let rx_buffer = esp_hal::dma_rx_stream_buffer!(RX_DMA_BUFFER_BYTES, esp_hal::dma::CHUNK_SIZE);
    let tx_buffer = esp_hal::dma_tx_stream_buffer!(TX_DMA_BUFFER_BYTES, esp_hal::dma::CHUNK_SIZE);

    let i2s = I2s::new(
        i2s0,
        dma,
        TdmConfig::new_tdm_philips()
            .with_signal_loopback(true)
            .with_sample_rate(Rate::from_hz(SAMPLE_RATE_HZ))
            .with_data_format(DataFormat::Data16Channel16)
            .with_channels(Channels::STEREO),
    )
    .expect("Failed to configure shared audio I2S0")
    .with_mclk(mclk)
    .into_async();

    let i2s_tx = i2s
        .i2s_tx
        .with_bclk(bclk)
        .with_ws(word_select)
        .with_dout(data_out)
        .build();
    let i2s_rx = i2s
        .i2s_rx
        .with_bclk(NoPin)
        .with_ws(NoPin)
        .with_din(data_in)
        .build();

    spawner.spawn(
        playback::playback_task(i2s_tx, tx_buffer, runtime)
            .expect("Failed to allocate audio TX task"),
    );

    {
        let mut transfer = i2s_rx
            .read(rx_buffer)
            .map_err(|(error, _, _)| error)
            .expect("Failed to start circular I2S RX DMA");

        ::log::info!(
            "Microphone DMA started: {} Hz, stereo, 16-bit",
            SAMPLE_RATE_HZ
        );

        let mut dma_drain = storage::FixedPsramBuffer::filled(RX_DMA_BUFFER_BYTES, 0u8);
        let mut samples = [0i16; SAMPLES_PER_BLOCK];
        let mut frame_index = 0usize;
        let mut peak_left = 0u16;
        let mut peak_right = 0u16;
        let mut first_block = true;

        loop {
            if transfer.wait_for_available_async().await.is_err() {
                panic!("I2S circular DMA read failed");
            }

            let available = transfer.available_bytes().min(RX_DMA_BUFFER_BYTES);
            if available == 0 {
                continue;
            }
            let count = transfer.pop(&mut dma_drain.as_mut_slice()[..available]);

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

                    if frame_index == FRAMES_PER_BLOCK {
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
