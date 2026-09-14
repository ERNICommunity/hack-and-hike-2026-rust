//! CPU1 tasks owning I2S0: one captures the microphones, one plays the
//! speaker queue.
//!
//! Both directions use streaming DMA: the peripheral writes to (or reads
//! from) a ring buffer continuously, and the tasks copy data out of (or into)
//! it whenever enough is available. The data sits in DMA memory in the codec
//! format, little-endian 16-bit samples.

use embassy_executor::Spawner;
use embassy_futures::yield_now;
use esp_hal::{
    Async,
    dma::{DmaRxStreamBuf, DmaTxStreamBuf},
    gpio::NoPin,
    i2s::master::{Channels, DataFormat, I2s, I2sRx, I2sTx, TdmConfig},
    time::Rate,
};
use log::{info, warn};

use super::{CHANNELS, FRAMES_PER_BLOCK, MicBlock, Resources, Runtime, SAMPLE_RATE_HZ};

/// Bytes of one sample in the I2S stream.
const BYTES_PER_SAMPLE: usize = size_of::<i16>();
/// Bytes of one stereo frame in the I2S stream.
const BYTES_PER_FRAME: usize = CHANNELS * BYTES_PER_SAMPLE;
/// DMA ring for captured audio: 32 KiB is half a second of slack.
const RX_DMA_BUFFER_BYTES: usize = 32 * 1024;
/// Bytes drained from the DMA ring per step, small enough for the task stack.
const RX_DRAIN_BYTES: usize = 1024;
/// esp-hal's streaming TX buffer needs at least four DMA descriptors.
const TX_DMA_BUFFER_BYTES: usize = 4 * esp_hal::dma::CHUNK_SIZE;
/// Bytes handed to the TX DMA per step: 16 ms of stereo 16-bit audio.
const TX_FILL_BYTES: usize = 1_024;
/// Samples handed to the TX DMA per step: [`TX_FILL_BYTES`] in 16-bit samples.
const TX_FILL_SAMPLES: usize = TX_FILL_BYTES / BYTES_PER_SAMPLE;
const _: () = assert!(RX_DRAIN_BYTES.is_multiple_of(BYTES_PER_FRAME));

/// Start audio on CPU1: the capture task, which spawns the playback task
/// once the shared I2S clock runs.
pub(crate) fn spawn(spawner: &Spawner, resources: Resources, runtime: Runtime) {
    spawner.spawn(capture_task(resources, *spawner, runtime).expect("audio task already spawned"));
}

/// Own I2S0 on CPU1. TX always runs because it is the BCLK/WS master; RX
/// follows the same clocks through signal loopback.
#[embassy_executor::task]
async fn capture_task(resources: Resources, spawner: Spawner, runtime: Runtime) {
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
        playback_task(i2s_tx, tx_buffer, runtime).expect("audio playback task already spawned"),
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
    /// The block being filled.
    block: MicBlock,
    /// Frames already in `block`.
    frames: usize,
    /// Sequence number of the last published block.
    sequence: u32,
    /// Blocks dropped so far because the queue was full.
    dropped_blocks: u32,
}

impl BlockBuilder {
    /// An empty builder.
    const fn new() -> Self {
        Self {
            block: MicBlock::SILENT,
            frames: 0,
            sequence: 0,
            dropped_blocks: 0,
        }
    }

    /// Add whole frames from `bytes`, publishing every block that fills up.
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

    /// Number the finished block, queue it and start the next one.
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

/// Play forever. A DMA error restarts the transfer instead of ending audio.
#[embassy_executor::task]
async fn playback_task(
    mut i2s_tx: I2sTx<'static, Async>,
    mut tx_buffer: DmaTxStreamBuf,
    runtime: Runtime,
) {
    let mut staging = [0u8; TX_FILL_BYTES];
    let mut pcm = [0i16; TX_FILL_SAMPLES];

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
                let samples = &pcm[..frames * CHANNELS];
                for (bytes, sample) in staging.chunks_exact_mut(BYTES_PER_SAMPLE).zip(samples) {
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
