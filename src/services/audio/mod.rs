//! CPU1-owned full-duplex audio service.
//!
//! I2S0 owns the shared microphone/speaker clock domain. RX continuously captures
//! the ES7210 microphones while TX continuously feeds the AW88298 amplifier.
//! CPU0 publishes semantic playback state through a replace-latest control; the
//! real-time synthesis/DMA state never leaves CPU1.

mod chime;
mod melody;

use core::sync::atomic::{AtomicU32, Ordering};

use embassy_executor::Spawner;
use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex,
    mutex::Mutex,
    signal::Signal,
};
use esp_hal::{
    Async,
    delay::Delay,
    gpio::NoPin,
    i2s::master::{Channels, Config as I2sConfig, DataFormat, I2s, I2sTx},
    peripherals::{DMA_CH0, GPIO0, GPIO13, GPIO14, GPIO33, GPIO34, I2S0},
    time::Rate,
};

use crate::{board, data_plane, diagnostics};
use chime::FlashChime;
use melody::MelodySynth;

pub const SAMPLE_RATE_HZ: u32 = 16_000;
pub const BLOCK_FRAMES: usize = 512;
pub const CHANNELS: usize = 2;
pub const BLOCK_SAMPLES: usize = BLOCK_FRAMES * CHANNELS;

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
// Render speaker data into a small staging block and feed it through `push`,
// whose esp-hal 1.1.x implementation correctly propagates TX-underrun errors.
// 1024 bytes is 16 ms of stereo 16-bit audio at 16 kHz, keeping controls snappy.
const PLAYBACK_FILL_BYTES: usize = 1_024;
const _: () = assert!(RX_PROCESS_CHUNK_BYTES % 4 == 0);
const _: () = assert!(TX_DMA_BUFFER_BYTES % 4 == 0);
const _: () = assert!(PLAYBACK_FILL_BYTES % 4 == 0);
const ES7210_ADDR: u8 = 0x40;
const AW88298_ADDR: u8 = 0x36;

// The one-shot is derived offline from the user-supplied MP3 and stored as
// flash-resident IMA ADPCM; decoding is incremental and allocation-free.

/// Valid melody tempo in quarter-note beats per minute.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TempoBpm(u16);

impl TempoBpm {
    pub const MIN: Self = Self(60);
    pub const DEFAULT: Self = Self(120);
    pub const MAX: Self = Self(180);

    pub const fn new(value: u16) -> Option<Self> {
        if value >= Self::MIN.0 && value <= Self::MAX.0 {
            Some(Self(value))
        } else {
            None
        }
    }

    pub const fn get(self) -> u16 {
        self.0
    }
}

/// Chromatic pitch transposition applied to the synthesized MIDI loop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PitchSemitones(i8);

impl PitchSemitones {
    pub const MIN: Self = Self(-12);
    pub const CENTER: Self = Self(0);
    pub const MAX: Self = Self(12);

    pub const fn new(value: i8) -> Option<Self> {
        if value >= Self::MIN.0 && value <= Self::MAX.0 {
            Some(Self(value))
        } else {
            None
        }
    }

    pub const fn get(self) -> i8 {
        self.0
    }
}

/// Complete continuous playback intent published by CPU0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlaybackSettings {
    pub melody_playing: bool,
    pub tempo: TempoBpm,
    pub pitch: PitchSemitones,
}

impl PlaybackSettings {
    pub const DEFAULT: Self = Self {
        melody_playing: false,
        tempo: TempoBpm::DEFAULT,
        pitch: PitchSemitones::CENTER,
    };
}

static PLAYBACK_SETTINGS: Signal<CriticalSectionRawMutex, PlaybackSettings> = Signal::new();
static ONE_SHOT_SEQUENCE: AtomicU32 = AtomicU32::new(0);

/// CPU0 command handle for the CPU1 audio-output service.
#[derive(Clone, Copy)]
pub struct PlaybackControl;

impl PlaybackControl {
    pub const fn from_static_service() -> Self {
        Self
    }

    pub fn set(self, settings: PlaybackSettings) {
        PLAYBACK_SETTINGS.signal(settings);
    }

    pub fn play_one_shot(self) {
        ONE_SHOT_SEQUENCE.fetch_add(1, Ordering::Release);
    }
}

/// CPU1-owned physical resources required by the shared audio service.
pub struct Resources {
    pub i2s0: I2S0<'static>,
    pub dma: DMA_CH0<'static>,
    pub mclk: GPIO0<'static>,
    pub bclk: GPIO34<'static>,
    pub word_select: GPIO33<'static>,
    pub data_in: GPIO14<'static>,
    pub data_out: GPIO13<'static>,
}

#[derive(Clone, Copy, Debug)]
pub struct AudioBlockInfo {
    pub sequence: u32,
    pub peak_left: u16,
    pub peak_right: u16,
}

struct LatestAudio {
    samples: [i16; BLOCK_SAMPLES],
    info: AudioBlockInfo,
}

impl LatestAudio {
    const fn new() -> Self {
        Self {
            samples: [0; BLOCK_SAMPLES],
            info: AudioBlockInfo {
                sequence: 0,
                peak_left: 0,
                peak_right: 0,
            },
        }
    }
}

static LATEST_AUDIO: Mutex<CriticalSectionRawMutex, LatestAudio> = Mutex::new(LatestAudio::new());

/// Configure ES7210 MIC1/MIC2 for stereo 16 kHz I2S input.
pub fn init_es7210<I2C>(i2c: &mut I2C) -> Result<(), I2C::Error>
where
    I2C: embedded_hal::i2c::I2c,
{
    board::power::enable_microphone(i2c)?;
    i2c.write(ES7210_ADDR, &[0x00, 0xFF])?;

    const ES7210_INIT: &[(u8, u8)] = &[
        (0x00, 0x41), (0x01, 0x1F), (0x06, 0x00), (0x07, 0x20),
        (0x08, 0x10), (0x09, 0x30), (0x0A, 0x30), (0x20, 0x0A),
        (0x21, 0x2A), (0x22, 0x0A), (0x23, 0x2A), (0x02, 0xC1),
        (0x04, 0x01), (0x05, 0x00), (0x11, 0x60), (0x40, 0x42),
        (0x41, 0x70), (0x42, 0x70), (0x43, 0x1B), (0x44, 0x1B),
        (0x45, 0x00), (0x46, 0x00), (0x47, 0x00), (0x48, 0x00),
        (0x49, 0x00), (0x4A, 0x00), (0x4B, 0x00), (0x4C, 0xFF),
        (0x01, 0x14),
    ];

    for &(register, value) in ES7210_INIT {
        i2c.write(ES7210_ADDR, &[register, value])?;
    }
    Ok(())
}

/// Release and configure the onboard AW88298 speaker amplifier for the same
/// 16 kHz, stereo, 16-bit I2S clock domain used by microphone capture.
pub fn init_aw88298<I2C>(i2c: &mut I2C, delay: &mut Delay) -> Result<(), I2C::Error>
where
    I2C: embedded_hal::i2c::I2c,
{
    board::io_expander::release_audio_amplifier(i2c, delay)?;

    // CoreS3 reference configuration from M5Unified. At 16 kHz the rate-table
    // index is 3, therefore register 0x06 is 0x14C3 for 16-bit stereo BCK mode.
    aw88298_write(i2c, 0x61, 0x0673)?; // boost disabled
    aw88298_write(i2c, 0x04, 0x4040)?; // I2S enabled, amplifier powered
    aw88298_write(i2c, 0x05, 0x0008)?; // unmuted
    aw88298_write(i2c, 0x06, 0x14C3)?; // 16 kHz, 16-bit x 2
    aw88298_write(i2c, 0x0C, 0x0064)?; // reference full-volume setting
    Ok(())
}

fn aw88298_write<I2C>(i2c: &mut I2C, register: u8, value: u16) -> Result<(), I2C::Error>
where
    I2C: embedded_hal::i2c::I2c,
{
    let [high, low] = value.to_be_bytes();
    i2c.write(AW88298_ADDR, &[register, high, low])
}

async fn publish(samples: &[i16; BLOCK_SAMPLES], peak_left: u16, peak_right: u16) -> u32 {
    let mut latest = LATEST_AUDIO.lock().await;
    latest.samples.copy_from_slice(samples);
    latest.info.sequence = latest.info.sequence.wrapping_add(1);
    latest.info.peak_left = peak_left;
    latest.info.peak_right = peak_right;
    latest.info.sequence
}

pub fn copy_latest_interleaved(out: &mut [i16; BLOCK_SAMPLES]) -> Option<AudioBlockInfo> {
    let latest = LATEST_AUDIO.try_lock().ok()?;
    if latest.info.sequence == 0 {
        return None;
    }
    out.copy_from_slice(&latest.samples);
    Some(latest.info)
}

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
pub async fn capture_task(resources: Resources, spawner: Spawner) {
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

    spawner
        .spawn(playback_task(i2s_tx, tx_buffer).expect("Failed to allocate speaker playback task"));

    let mut transfer = i2s_rx
        .read_dma_circular_async(rx_buffer)
        .expect("Failed to start circular I2S RX DMA");

    ::log::info!(
        "Audio DMA started: {} Hz, stereo, 16-bit, mic + speaker full duplex",
        SAMPLE_RATE_HZ
    );

    let mut dma_drain = data_plane::FixedPsramBuffer::filled(RX_DMA_BUFFER_BYTES, 0u8);
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
                    let sequence = publish(&samples, peak_left, peak_right).await;
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

#[embassy_executor::task]
async fn playback_task(i2s_tx: I2sTx<'static, Async>, tx_buffer: &'static mut [u8]) {
    // Circular TX starts reading immediately. Silence the complete ring first
    // so startup can never replay uninitialized/stale bytes before the task's
    // first refill.
    tx_buffer.fill(0);

    let mut transfer = i2s_tx
        .write_dma_circular_async(tx_buffer)
        .expect("Failed to start circular I2S TX DMA");
    let mut engine = PlaybackEngine::new();
    let mut settings = PlaybackSettings::DEFAULT;
    let mut one_shot_seen = ONE_SHOT_SEQUENCE.load(Ordering::Acquire);
    let mut staging = [0u8; PLAYBACK_FILL_BYTES];
    let mut staging_offset = staging.len();

    loop {
        if staging_offset == staging.len() {
            if let Some(next) = PLAYBACK_SETTINGS.try_take() {
                if next.melody_playing && !settings.melody_playing {
                    engine.melody.restart();
                }
                settings = next;
            }

            let one_shot_sequence = ONE_SHOT_SEQUENCE.load(Ordering::Acquire);
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