use core::cell::RefCell;

use critical_section::Mutex;
use esp_hal::{
    delay::Delay,
    i2s::master::{Channels, Config as I2sConfig, DataFormat, I2s},
    peripherals::{DMA_CH0, GPIO0, GPIO14, GPIO33, GPIO34, I2S0},
    time::Rate,
};

pub const SAMPLE_RATE_HZ: u32 = 16_000;
pub const BLOCK_FRAMES: usize = 512;
pub const CHANNELS: usize = 2;
pub const BLOCK_SAMPLES: usize = BLOCK_FRAMES * CHANNELS;

const DMA_BUFFER_BYTES: usize = 32 * 1024;

const AXP2101_ADDR: u8 = 0x34;
const AW9523_ADDR: u8 = 0x58;
const ES7210_ADDR: u8 = 0x40;

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

static LATEST_AUDIO: Mutex<RefCell<LatestAudio>> = Mutex::new(RefCell::new(LatestAudio::new()));

fn update_register_bits<I2C>(
    i2c: &mut I2C,
    address: u8,
    register: u8,
    mask: u8,
    value: u8,
) -> Result<(), I2C::Error>
where
    I2C: embedded_hal::i2c::I2c,
{
    let mut current = [0u8; 1];
    i2c.write_read(address, &[register], &mut current)?;
    let next = (current[0] & !mask) | (value & mask);
    i2c.write(address, &[register, next])
}

/// Powers the microphone path and configures the ES7210 for the two onboard
/// microphones. The register sequence follows the CoreS3 setup used by
/// M5Unified: MIC1/MIC2 enabled, MIC3/MIC4 powered down, stereo I2S output.
pub fn init_es7210<I2C>(i2c: &mut I2C, delay: &mut Delay) -> Result<(), I2C::Error>
where
    I2C: embedded_hal::i2c::I2c,
{
    // CoreS3-Lite microphone power: AXP2101 ALDO2 = 3.3 V and enabled.
    i2c.write(AXP2101_ADDR, &[0x93, 0x1C])?;
    update_register_bits(i2c, AXP2101_ADDR, 0x90, 1 << 1, 1 << 1)?;

    // CoreS3(-Lite) routes AW9523 P0_2 to the AW88298 amplifier reset.
    // Keep the amplifier released during board audio bring-up. AW9523 direction:
    // 0 = output, 1 = input.
    update_register_bits(i2c, AW9523_ADDR, 0x04, 1 << 2, 0)?;
    update_register_bits(i2c, AW9523_ADDR, 0x02, 1 << 2, 1 << 2)?;
    delay.delay_millis(10u32);

    // Reset before programming the codec.
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

fn publish(samples: &[i16; BLOCK_SAMPLES], peak_left: u16, peak_right: u16) -> u32 {
    critical_section::with(|cs| {
        let mut latest = LATEST_AUDIO.borrow(cs).borrow_mut();
        latest.samples.copy_from_slice(samples);
        latest.info.sequence = latest.info.sequence.wrapping_add(1);
        latest.info.peak_left = peak_left;
        latest.info.peak_right = peak_right;
        latest.info.sequence
    })
}

/// Copies the newest complete 512-frame stereo block into `out`.
///
/// This is the hand-off point intended for the later waveform renderer. It
/// copies under a short critical section so the consumer never observes a
/// half-published audio block. Returns `None` until the first DMA block arrives
/// or when `out` is too small.
pub fn copy_latest_interleaved(out: &mut [i16]) -> Option<AudioBlockInfo> {
    if out.len() < BLOCK_SAMPLES {
        return None;
    }

    critical_section::with(|cs| {
        let latest = LATEST_AUDIO.borrow(cs).borrow();
        if latest.info.sequence == 0 {
            return None;
        }

        out[..BLOCK_SAMPLES].copy_from_slice(&latest.samples);
        Some(latest.info)
    })
}

#[embassy_executor::task]
pub async fn capture_task(
    i2s0: I2S0<'static>,
    dma_channel: DMA_CH0<'static>,
    mclk: GPIO0<'static>,
    bclk: GPIO34<'static>,
    ws: GPIO33<'static>,
    din: GPIO14<'static>,
) {
    let (rx_buffer, rx_descriptors, _, _) = esp_hal::dma_buffers!(DMA_BUFFER_BYTES, 0);

    let i2s = I2s::new(
        i2s0,
        dma_channel,
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
        .with_ws(ws)
        .with_din(din)
        .build(rx_descriptors);

    let mut transfer = i2s_rx
        .read_dma_circular_async(rx_buffer)
        .expect("Failed to start circular I2S RX DMA");

    ::log::info!(
        "Audio DMA started: {} Hz, stereo, 16-bit, {} frames/block",
        SAMPLE_RATE_HZ,
        BLOCK_FRAMES
    );

    // `I2sReadDmaTransferAsync::pop()` in esp-hal 1.1.x requires the
    // destination to be large enough for *all bytes currently available* in
    // the circular DMA ring. A BLOCK_BYTES-sized destination (2048 bytes) is
    // therefore not sufficient when a 4092-byte DMA descriptor, or several
    // descriptors, have completed before the task runs.
    //
    // Size this drain buffer exactly like the DMA ring so any non-late amount
    // reported by the driver can always be consumed in one pop(). Allocate it
    // once on the heap so the Embassy task future itself does not contain a
    // 32 KiB inline array.
    let mut dma_drain = alloc::vec![0u8; DMA_BUFFER_BYTES];

    let mut samples = [0i16; BLOCK_SAMPLES];
    let mut frame_index = 0usize;
    let mut peak_left = 0u16;
    let mut peak_right = 0u16;
    let mut first_block = true;

    loop {
        let count = transfer
            .pop(dma_drain.as_mut_slice())
            .await
            .expect("I2S circular DMA read failed");

        // The configured stereo 16-bit I2S stream is four bytes per frame and
        // esp-hal's I2S DMA buffers/descriptors are 4-byte aligned.
        for frame_bytes in dma_drain[..count].chunks_exact(4) {
            let left = i16::from_le_bytes([frame_bytes[0], frame_bytes[1]]);
            let right = i16::from_le_bytes([frame_bytes[2], frame_bytes[3]]);

            samples[frame_index * 2] = left;
            samples[frame_index * 2 + 1] = right;
            peak_left = peak_left.max(left.unsigned_abs());
            peak_right = peak_right.max(right.unsigned_abs());
            frame_index += 1;

            if frame_index == BLOCK_FRAMES {
                let sequence = publish(&samples, peak_left, peak_right);

                if first_block {
                    first_block = false;
                    ::log::info!(
                        "First audio block captured: seq={}, peak L={}, R={}",
                        sequence,
                        peak_left,
                        peak_right
                    );
                }

                // Start assembling the next 512-frame visualization block.
                frame_index = 0;
                peak_left = 0;
                peak_right = 0;
            }
        }
    }
}
