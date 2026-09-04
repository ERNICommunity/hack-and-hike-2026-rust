use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, mutex::Mutex};
use esp_hal::{
    delay::Delay,
    i2s::master::{Channels, Config as I2sConfig, DataFormat, I2s},
    peripherals::{DMA_CH0, GPIO0, GPIO14, GPIO33, GPIO34, I2S0},
    time::Rate,
};

use crate::{board, data_plane, diagnostics};

pub const SAMPLE_RATE_HZ: u32 = 16_000;
pub const BLOCK_FRAMES: usize = 512;
pub const CHANNELS: usize = 2;
pub const BLOCK_SAMPLES: usize = BLOCK_FRAMES * CHANNELS;

// Keep the generous ring that already proved stable with radio + dual-core
// scheduling. The actual DMA ring/descriptors remain in internal DMA-capable
// RAM; only the CPU-side drain scratch is allocated from PSRAM.
const DMA_BUFFER_BYTES: usize = 32 * 1024;

const ES7210_ADDR: u8 = 0x40;

/// CPU1-owned physical resources required by the audio acquisition service.
///
/// ES7210 register configuration remains in this module, while this bundle
/// describes the I2S/DMA/GPIO resources consumed by `capture_task`.
pub struct Resources {
    pub i2s0: I2S0<'static>,
    pub dma: DMA_CH0<'static>,
    pub mclk: GPIO0<'static>,
    pub bclk: GPIO34<'static>,
    pub word_select: GPIO33<'static>,
    pub data_in: GPIO14<'static>,
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

// Unlike critical_section::Mutex, Embassy's async mutex only uses the raw
// critical-section mutex briefly to update lock state. The 2 KiB PCM copies do
// not run with interrupts / the other core excluded.
static LATEST_AUDIO: Mutex<CriticalSectionRawMutex, LatestAudio> = Mutex::new(LatestAudio::new());

/// Power the microphone path and configure ES7210 MIC1/MIC2 for stereo I2S.
pub fn init_es7210<I2C>(i2c: &mut I2C, delay: &mut Delay) -> Result<(), I2C::Error>
where
    I2C: embedded_hal::i2c::I2c,
{
    board::power::enable_microphone(i2c)?;
    board::io_expander::release_audio_amplifier(i2c, delay)?;

    i2c.write(ES7210_ADDR, &[0x00, 0xFF])?;

    const ES7210_INIT: &[(u8, u8)] = &[
        (0x00, 0x41), // RESET_CTL
        (0x01, 0x1F), // CLK_ON_OFF
        (0x06, 0x00), // DIGITAL_PDN
        (0x07, 0x20), // ADC_OSR
        (0x08, 0x10), // MODE_CFG
        (0x09, 0x30), // TCT0_CHPINI
        (0x0A, 0x30), // TCT1_CHPINI
        (0x20, 0x0A), // ADC34_HPF2
        (0x21, 0x2A), // ADC34_HPF1
        (0x22, 0x0A), // ADC12_HPF2
        (0x23, 0x2A), // ADC12_HPF1
        (0x02, 0xC1),
        (0x04, 0x01),
        (0x05, 0x00),
        (0x11, 0x60),
        (0x40, 0x42), // ANALOG_SYS
        (0x41, 0x70), // MICBIAS12
        (0x42, 0x70), // MICBIAS34
        (0x43, 0x1B), // MIC1_GAIN
        (0x44, 0x1B), // MIC2_GAIN
        (0x45, 0x00), // MIC3_GAIN
        (0x46, 0x00), // MIC4_GAIN
        (0x47, 0x00), // MIC1_LP
        (0x48, 0x00), // MIC2_LP
        (0x49, 0x00), // MIC3_LP
        (0x4A, 0x00), // MIC4_LP
        (0x4B, 0x00), // MIC12_PDN
        (0x4C, 0xFF), // MIC34_PDN
        (0x01, 0x14), // CLK_ON_OFF
    ];

    for &(register, value) in ES7210_INIT {
        i2c.write(ES7210_ADDR, &[register, value])?;
    }

    Ok(())
}

async fn publish(samples: &[i16; BLOCK_SAMPLES], peak_left: u16, peak_right: u16) -> u32 {
    let mut latest = LATEST_AUDIO.lock().await;
    latest.samples.copy_from_slice(samples);
    latest.info.sequence = latest.info.sequence.wrapping_add(1);
    latest.info.peak_left = peak_left;
    latest.info.peak_right = peak_right;
    latest.info.sequence
}

/// Copy the newest complete stereo block without waiting. If CPU1 is in the
/// middle of publishing, CPU0 simply skips this presentation tick and will pick
/// up a newer block next time.
pub fn copy_latest_interleaved(out: &mut [i16; BLOCK_SAMPLES]) -> Option<AudioBlockInfo> {
    let latest = LATEST_AUDIO.try_lock().ok()?;
    if latest.info.sequence == 0 {
        return None;
    }

    out.copy_from_slice(&latest.samples);
    Some(latest.info)
}

#[embassy_executor::task]
pub async fn capture_task(resources: Resources) {
    let Resources {
        i2s0,
        dma,
        mclk,
        bclk,
        word_select,
        data_in,
    } = resources;

    let (rx_buffer, rx_descriptors, _, _) = esp_hal::dma_buffers!(DMA_BUFFER_BYTES, 0);

    let i2s = I2s::new(
        i2s0,
        dma,
        I2sConfig::new_tdm_philips()
            .with_sample_rate(Rate::from_hz(SAMPLE_RATE_HZ))
            .with_data_format(DataFormat::Data16Channel16)
            .with_channels(Channels::STEREO),
    )
    .expect("Failed to configure I2S0")
    .with_mclk(mclk)
    .into_async();

    let i2s_rx = i2s
        .i2s_rx
        .with_bclk(bclk)
        .with_ws(word_select)
        .with_din(data_in)
        .build(rx_descriptors);

    let mut transfer = i2s_rx
        .read_dma_circular_async(rx_buffer)
        .expect("Failed to start circular I2S RX DMA");

    ::log::info!(
        "Audio DMA started: {} Hz, stereo, 16-bit, {} frames/block",
        SAMPLE_RATE_HZ,
        BLOCK_FRAMES
    );

    // pop() requires enough destination space for every byte currently
    // available in the circular ring, hence a drain buffer the size of the ring.
    // This destination is CPU-accessed only, so keep its 32 KiB backing storage
    // in PSRAM while the actual DMA ring/descriptors above remain internal.
    let mut dma_drain = data_plane::FixedPsramBuffer::filled(DMA_BUFFER_BYTES, 0u8);

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

        if count == DMA_BUFFER_BYTES {
            diagnostics::record_audio_full_drain();
        }

        for frame_bytes in dma_drain.as_slice()[..count].chunks_exact(4) {
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
    }
}
