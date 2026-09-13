//! CPU1 owner of I2S0: brings up the shared clock domain and captures the
//! microphone.

use embassy_executor::Spawner;
use embassy_futures::yield_now;
use esp_hal::{
    Async,
    dma::DmaRxStreamBuf,
    gpio::NoPin,
    i2s::master::{Channels, DataFormat, I2s, I2sRx, TdmConfig},
    time::Rate,
};
use log::{info, warn};

use super::{
    CHANNELS, FRAMES_PER_BLOCK, Resources, SAMPLE_RATE_HZ,
    channels::{MicBlock, Runtime},
    playback,
};

const BYTES_PER_SAMPLE: usize = 2;
const BYTES_PER_FRAME: usize = CHANNELS * BYTES_PER_SAMPLE;
/// DMA ring for captured audio: 32 KiB is half a second of slack.
const RX_DMA_BUFFER_BYTES: usize = 32 * 1024;
/// Bytes drained from the DMA ring per step, small enough for the task stack.
const RX_DRAIN_BYTES: usize = 1024;
/// esp-hal's streaming TX buffer needs at least four DMA descriptors.
const TX_DMA_BUFFER_BYTES: usize = 4 * esp_hal::dma::CHUNK_SIZE;
const _: () = assert!(RX_DRAIN_BYTES.is_multiple_of(BYTES_PER_FRAME));

/// Own I2S0 on CPU1. TX always runs because it is the BCLK/WS master; RX
/// follows the same clocks through signal loopback.
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
    .expect("I2S0 configuration is valid")
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
            .expect("audio playback task already spawned"),
    );
    info!("Audio started: {} Hz, stereo, 16-bit", SAMPLE_RATE_HZ);

    capture_blocks(i2s_rx, rx_buffer, runtime).await
}

/// Capture forever. A DMA error restarts the transfer instead of ending audio.
async fn capture_blocks(
    mut i2s_rx: I2sRx<'static, Async>,
    mut rx_buffer: DmaRxStreamBuf,
    runtime: Runtime,
) -> ! {
    let mut builder = BlockBuilder::new();
    let mut drain = [0u8; RX_DRAIN_BYTES];

    loop {
        let mut transfer = i2s_rx
            .read(rx_buffer)
            .map_err(|(error, _, _)| error)
            .expect("I2S RX DMA start failed");

        let error = loop {
            if let Err(error) = transfer.wait_for_available_async().await {
                break error;
            }
            loop {
                let count = transfer.pop(&mut drain);
                if count == 0 {
                    break;
                }
                builder.push_bytes(&drain[..count], runtime);
                yield_now().await;
            }
        };

        warn!("Microphone DMA error {:?}; restarting capture", error);
        (i2s_rx, rx_buffer) = transfer.stop();
    }
}

/// Assembles raw DMA bytes into complete microphone blocks.
struct BlockBuilder {
    block: MicBlock,
    frames: usize,
    sequence: u32,
    dropped_blocks: u32,
}

impl BlockBuilder {
    const fn new() -> Self {
        Self {
            block: MicBlock::SILENT,
            frames: 0,
            sequence: 0,
            dropped_blocks: 0,
        }
    }

    fn push_bytes(&mut self, bytes: &[u8], runtime: Runtime) {
        for frame in bytes.chunks_exact(BYTES_PER_FRAME) {
            let left = i16::from_le_bytes([frame[0], frame[1]]);
            let right = i16::from_le_bytes([frame[2], frame[3]]);
            let index = self.frames * CHANNELS;
            self.block.samples[index] = left;
            self.block.samples[index + 1] = right;
            self.block.info.peak_left = self.block.info.peak_left.max(left.unsigned_abs());
            self.block.info.peak_right = self.block.info.peak_right.max(right.unsigned_abs());
            self.frames += 1;

            if self.frames == FRAMES_PER_BLOCK {
                self.publish(runtime);
            }
        }
    }

    fn publish(&mut self, runtime: Runtime) {
        self.sequence = self.sequence.wrapping_add(1);
        if runtime.microphone_queue_is_full() {
            self.dropped_blocks = self.dropped_blocks.wrapping_add(1);
        }
        self.block.info.sequence = self.sequence;
        self.block.info.dropped_blocks = self.dropped_blocks;
        runtime.publish_microphone_block(self.block);

        self.frames = 0;
        self.block.info.peak_left = 0;
        self.block.info.peak_right = 0;
    }
}
