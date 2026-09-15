//! The CPU1 tasks that own I2S0. One task captures the microphones. The
//! other task plays the samples from the speaker queue.
//!
//! I2S (Inter-IC Sound) is the bus that carries audio samples between the
//! ESP32-S3 and the codecs. Both directions use streaming DMA (direct memory
//! access). For the microphones, the peripheral writes into a DMA ring all
//! the time, and the capture task copies the data out of it. For the
//! speaker, the playback task copies data into a DMA ring, and the peripheral
//! reads from it all the time. In DMA memory, the data has the codec format:
//! 16-bit samples, least significant byte first (little-endian).

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
/// Size of the DMA ring for captured audio. 32 KiB are 8,192 stereo frames,
/// about half a second. The capture task can fall behind by that much before
/// data is lost.
const RX_DMA_BUFFER_BYTES: usize = 32 * 1024;
/// Bytes that the capture task copies out of the DMA ring in one step. This
/// buffer is small, so it fits on the task stack.
const RX_DRAIN_BYTES: usize = 1024;
/// Size of the DMA buffer for playback: four DMA descriptors of
/// `CHUNK_SIZE` (4,092) bytes each. esp-hal's streaming TX (transmit) buffer
/// needs at least four descriptors. A DMA descriptor is one block of the
/// buffer.
const TX_DMA_BUFFER_BYTES: usize = 4 * esp_hal::dma::CHUNK_SIZE;
/// Bytes that the playback task gives to the TX DMA in one step: 256 stereo
/// frames of 16-bit samples, 16 ms of audio.
const TX_FILL_BYTES: usize = 1_024;
/// Samples that the playback task gives to the TX DMA in one step:
/// [`TX_FILL_BYTES`] as 16-bit samples.
const TX_FILL_SAMPLES: usize = TX_FILL_BYTES / BYTES_PER_SAMPLE;
// Compile-time check: a full step of the capture task holds whole frames. A
// shorter read can still end inside a frame. `BlockBuilder` handles that.
const _: () = assert!(RX_DRAIN_BYTES.is_multiple_of(BYTES_PER_FRAME));

/// Start audio on CPU1. This spawns the capture task. The capture task
/// configures I2S0 and then spawns the playback task.
///
/// # Panics
///
/// When the capture task already runs.
pub(crate) fn spawn(spawner: &Spawner, resources: Resources, runtime: Runtime) {
    spawner.spawn(capture_task(resources, *spawner, runtime).expect("audio task already spawned"));
}

/// Own I2S0 on CPU1, spawn the playback task and capture forever.
///
/// The TX (transmit) side always runs, because it generates the bit clock
/// (BCLK) and the word select (WS) signal. The RX (receive) side has no
/// clock pins. It uses the same clocks through signal loopback inside the
/// peripheral.
///
/// # Panics
///
/// When esp-hal rejects the I2S configuration, or the playback task already
/// runs.
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

/// Capture forever. After a DMA error, the transfer starts again, and audio
/// continues.
///
/// After a restart, the builder drops the bytes of an unfinished frame. It
/// keeps the complete frames of the block in progress. So that block
/// contains audio from before and after the restart.
///
/// # Panics
///
/// When the DMA transfer does not start.
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
        builder.drop_partial_frame();
    }
}

/// Builds complete microphone blocks from the raw bytes of the DMA ring.
struct BlockBuilder {
    /// The block that is being filled.
    block: MicBlock,
    /// Frames already in `block`.
    frames: usize,
    /// Sequence number of the last published block. The first block gets
    /// number 1.
    sequence: u32,
    /// Blocks dropped so far because the queue was full.
    dropped_blocks: u32,
    /// The first bytes of a frame whose other bytes are not read yet. A read
    /// from the DMA ring can end in the middle of a frame.
    partial: [u8; BYTES_PER_FRAME],
    /// Number of bytes in `partial`. Between calls, always less than
    /// [`BYTES_PER_FRAME`].
    partial_len: usize,
}

impl BlockBuilder {
    /// An empty builder.
    const fn new() -> Self {
        Self {
            block: MicBlock::SILENT,
            frames: 0,
            sequence: 0,
            dropped_blocks: 0,
            partial: [0; BYTES_PER_FRAME],
            partial_len: 0,
        }
    }

    /// Add the frames in `bytes`, and publish every block that becomes full.
    ///
    /// A read from the DMA ring does not always end at a frame boundary.
    /// First, bytes from `bytes` complete the unfinished frame from the last
    /// call. At the end, the bytes of a new unfinished frame stay in
    /// `partial` for the next call. So left and right samples never swap.
    fn push_bytes(&mut self, mut bytes: &[u8], runtime: Runtime) {
        if self.partial_len > 0 {
            let take = (BYTES_PER_FRAME - self.partial_len).min(bytes.len());
            self.partial[self.partial_len..self.partial_len + take].copy_from_slice(&bytes[..take]);
            self.partial_len += take;
            bytes = &bytes[take..];
            if self.partial_len < BYTES_PER_FRAME {
                return;
            }
            self.partial_len = 0;
            let frame = self.partial;
            self.push_frame(&frame, runtime);
        }

        let mut frames = bytes.chunks_exact(BYTES_PER_FRAME);
        for frame in &mut frames {
            self.push_frame(frame, runtime);
        }
        let rest = frames.remainder();
        self.partial[..rest.len()].copy_from_slice(rest);
        self.partial_len = rest.len();
    }

    /// Forget the bytes of an unfinished frame. Call it when the capture
    /// restarts, because the next bytes then start a new frame. The complete
    /// frames in the block stay.
    fn drop_partial_frame(&mut self) {
        self.partial_len = 0;
    }

    /// Add one frame of [`BYTES_PER_FRAME`] (4) bytes: the left sample, then
    /// the right sample, each little-endian. Publish the block when it
    /// becomes full.
    fn push_frame(&mut self, frame: &[u8], runtime: Runtime) {
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

    /// Give the full block the next sequence number, put it into the queue,
    /// and start the next block.
    ///
    /// When the queue is full, the oldest unread block is dropped. The new
    /// block already counts it in `dropped_blocks`. The samples are not
    /// cleared: the next block overwrites all of them.
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

/// Play forever. After a DMA error, the transfer starts again, and audio
/// continues.
///
/// The task takes up to 256 frames from the speaker queue in one step. When
/// the queue has fewer frames, the rest of the step is silence (zeros).
///
/// # Panics
///
/// When the DMA transfer does not start.
#[embassy_executor::task]
async fn playback_task(
    mut i2s_tx: I2sTx<'static, Async>,
    mut tx_buffer: DmaTxStreamBuf,
    runtime: Runtime,
) {
    // `staging` holds the bytes of one step. `staged` counts how many of them
    // the DMA buffer has already accepted.
    let mut staging = [0u8; TX_FILL_BYTES];
    // PCM (pulse-code modulation): plain samples, read from the queue.
    let mut pcm = [0i16; TX_FILL_SAMPLES];

    loop {
        // Fill the whole DMA buffer with silence before the transfer starts.
        // So the amplifier never plays old data.
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
            // When the DMA has sent one descriptor, it raises an EOF
            // (end-of-data) interrupt, and about 4 KiB become free at once.
            // Fill all free space before the task waits for the next EOF.
            if transfer.available_bytes() == 0 {
                if let Err(error) = transfer.wait_for_available_async().await {
                    break error;
                }
                continue;
            }

            // All staged bytes are in the DMA buffer: prepare the next step.
            // Bytes without a frame from the queue stay 0, which is silence.
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
