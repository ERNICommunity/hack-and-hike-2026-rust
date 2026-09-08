//! CPU1-owned full-duplex audio service.
//!
//! I2S0 owns the shared microphone/speaker clock domain. RX continuously captures
//! the ES7210 microphones while TX continuously feeds the AW88298 amplifier.
//! CPU0 publishes semantic playback state through a replace-latest control; the
//! real-time synthesis/DMA state never leaves CPU1.

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

pub const SAMPLE_RATE_HZ: u32 = 16_000;
pub const BLOCK_FRAMES: usize = 512;
pub const CHANNELS: usize = 2;
pub const BLOCK_SAMPLES: usize = BLOCK_FRAMES * CHANNELS;

const RX_DMA_BUFFER_BYTES: usize = 32 * 1024;
const TX_DMA_BUFFER_BYTES: usize = 8 * 1024;
const ES7210_ADDR: u8 = 0x40;
const AW88298_ADDR: u8 = 0x36;
const CHIME_WAV: &[u8] = include_bytes!("../assets/speaker_chime.wav");
const CHIME_PCM_OFFSET: usize = 44;

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

/// Chromatic pitch transposition applied to the synthesized melody.
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
        esp_hal::dma_buffers!(RX_DMA_BUFFER_BYTES, TX_DMA_BUFFER_BYTES);

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

#[embassy_executor::task]
async fn playback_task(i2s_tx: I2sTx<'static, Async>, tx_buffer: &'static mut [u8]) {
    let mut transfer = i2s_tx
        .write_dma_circular_async(tx_buffer)
        .expect("Failed to start circular I2S TX DMA");
    let mut engine = PlaybackEngine::new();
    let mut settings = PlaybackSettings::DEFAULT;
    let mut one_shot_seen = ONE_SHOT_SEQUENCE.load(Ordering::Acquire);

    loop {
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

        if transfer
            .push_with(|bytes| engine.fill(bytes, settings))
            .await
            .is_err()
        {
            diagnostics::record_audio_playback_error();
            panic!("I2S circular DMA write failed");
        }
    }
}

struct PlaybackEngine {
    melody: MelodySynth,
    chime: FlashChime,
}

impl PlaybackEngine {
    const fn new() -> Self {
        Self {
            melody: MelodySynth::new(),
            chime: FlashChime::new(),
        }
    }

    fn fill(&mut self, bytes: &mut [u8], settings: PlaybackSettings) -> usize {
        let usable = bytes.len() & !3;
        for frame in bytes[..usable].chunks_exact_mut(4) {
            let melody = if settings.melody_playing {
                self.melody.next_sample(settings.tempo, settings.pitch)
            } else {
                0
            };
            let sample = saturating_mix(melody, self.chime.next_sample());
            let encoded = sample.to_le_bytes();
            frame[0] = encoded[0];
            frame[1] = encoded[1];
            frame[2] = encoded[0];
            frame[3] = encoded[1];
        }
        usable
    }
}

fn saturating_mix(a: i16, b: i16) -> i16 {
    i32::from(a)
        .saturating_add(i32::from(b))
        .clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16
}

struct FlashChime {
    byte_index: usize,
    playing: bool,
}

impl FlashChime {
    const fn new() -> Self {
        Self {
            byte_index: CHIME_PCM_OFFSET,
            playing: false,
        }
    }

    fn restart(&mut self) {
        self.byte_index = CHIME_PCM_OFFSET;
        self.playing = true;
    }

    fn next_sample(&mut self) -> i16 {
        if !self.playing || self.byte_index + 1 >= CHIME_WAV.len() {
            self.playing = false;
            return 0;
        }
        let sample = i16::from_le_bytes([
            CHIME_WAV[self.byte_index],
            CHIME_WAV[self.byte_index + 1],
        ]);
        self.byte_index += 2;
        sample
    }
}

#[derive(Clone, Copy)]
struct MelodyNote {
    midi: i8,
    eighths: u8,
}

const REST: i8 = -1;
const MELODY: [MelodyNote; 16] = [
    MelodyNote { midi: 60, eighths: 1 },
    MelodyNote { midi: 64, eighths: 1 },
    MelodyNote { midi: 67, eighths: 1 },
    MelodyNote { midi: 72, eighths: 2 },
    MelodyNote { midi: REST, eighths: 1 },
    MelodyNote { midi: 67, eighths: 1 },
    MelodyNote { midi: 64, eighths: 1 },
    MelodyNote { midi: 62, eighths: 2 },
    MelodyNote { midi: 65, eighths: 1 },
    MelodyNote { midi: 69, eighths: 1 },
    MelodyNote { midi: 67, eighths: 1 },
    MelodyNote { midi: 64, eighths: 2 },
    MelodyNote { midi: 62, eighths: 1 },
    MelodyNote { midi: 60, eighths: 1 },
    MelodyNote { midi: REST, eighths: 1 },
    MelodyNote { midi: 60, eighths: 2 },
];

struct MelodySynth {
    phase: u32,
    note_index: usize,
    sample_in_note: u32,
}

impl MelodySynth {
    const fn new() -> Self {
        Self {
            phase: 0,
            note_index: 0,
            sample_in_note: 0,
        }
    }

    fn restart(&mut self) {
        self.phase = 0;
        self.note_index = 0;
        self.sample_in_note = 0;
    }

    fn next_sample(&mut self, tempo: TempoBpm, pitch: PitchSemitones) -> i16 {
        let note = MELODY[self.note_index];
        let total = note_samples(note.eighths, tempo);
        if self.sample_in_note >= total {
            self.note_index = (self.note_index + 1) % MELODY.len();
            self.sample_in_note = 0;
            self.phase = 0;
            return self.next_sample(tempo, pitch);
        }

        let position = self.sample_in_note;
        self.sample_in_note += 1;
        if note.midi == REST {
            return 0;
        }

        let midi = i16::from(note.midi) + i16::from(pitch.get());
        let step = phase_step(midi);
        let wave = i32::from(SINE_256[(self.phase >> 24) as usize]);
        self.phase = self.phase.wrapping_add(step);

        let envelope = envelope_q15(position, total);
        let sample = (((wave * 6000) / 32767) * envelope) / 32767;
        sample as i16
    }
}

fn note_samples(eighths: u8, tempo: TempoBpm) -> u32 {
    let numerator = u64::from(SAMPLE_RATE_HZ) * 60 * u64::from(eighths);
    (numerator / (u64::from(tempo.get()) * 2)).max(1) as u32
}

fn envelope_q15(position: u32, total: u32) -> i32 {
    const ATTACK: u32 = SAMPLE_RATE_HZ / 200; // 5 ms
    const RELEASE: u32 = SAMPLE_RATE_HZ / 50; // 20 ms
    const GAP: u32 = SAMPLE_RATE_HZ / 200; // 5 ms silence between notes

    let sounding = total.saturating_sub(GAP);
    if position >= sounding {
        return 0;
    }
    let attack = ((position.min(ATTACK) * 32767) / ATTACK.max(1)) as i32;
    let remaining = sounding.saturating_sub(position);
    let release = ((remaining.min(RELEASE) * 32767) / RELEASE.max(1)) as i32;
    attack.min(release).clamp(0, 32767)
}

fn phase_step(midi: i16) -> u32 {
    const MIDI_MIN: i16 = 48;
    const MIDI_MAX: i16 = 84;
    const STEPS: [u32; 37] = [
        35114789, 37202823, 39415018, 41758757, 44241862, 46872620, 49659811,
        52612737, 55741253, 59055800, 62567441, 66287895, 70229578, 74405646,
        78830036, 83517514, 88483724, 93745240, 99319622, 105225474, 111482506,
        118111601, 125134882, 132575789, 140459156, 148811292, 157660072,
        167035027, 176967447, 187490479, 198639243, 210450947, 222965012,
        236223201, 250269764, 265151578, 280918312,
    ];
    STEPS[(midi.clamp(MIDI_MIN, MIDI_MAX) - MIDI_MIN) as usize]
}

const SINE_256: [i16; 256] = [
    0,804,1608,2410,3212,4011,4808,5602,6393,7179,7962,8739,9512,10278,11039,11793,
    12539,13279,14010,14732,15446,16151,16846,17530,18204,18868,19519,20159,20787,21403,22005,22594,
    23170,23731,24279,24811,25329,25832,26319,26790,27245,27683,28105,28510,28898,29268,29621,29956,
    30273,30571,30852,31113,31356,31580,31785,31971,32137,32285,32412,32521,32609,32678,32728,32757,
    32767,32757,32728,32678,32609,32521,32412,32285,32137,31971,31785,31580,31356,31113,30852,30571,
    30273,29956,29621,29268,28898,28510,28105,27683,27245,26790,26319,25832,25329,24811,24279,23731,
    23170,22594,22005,21403,20787,20159,19519,18868,18204,17530,16846,16151,15446,14732,14010,13279,
    12539,11793,11039,10278,9512,8739,7962,7179,6393,5602,4808,4011,3212,2410,1608,804,
    0,-804,-1608,-2410,-3212,-4011,-4808,-5602,-6393,-7179,-7962,-8739,-9512,-10278,-11039,-11793,
    -12539,-13279,-14010,-14732,-15446,-16151,-16846,-17530,-18204,-18868,-19519,-20159,-20787,-21403,-22005,-22594,
    -23170,-23731,-24279,-24811,-25329,-25832,-26319,-26790,-27245,-27683,-28105,-28510,-28898,-29268,-29621,-29956,
    -30273,-30571,-30852,-31113,-31356,-31580,-31785,-31971,-32137,-32285,-32412,-32521,-32609,-32678,-32728,-32757,
    -32767,-32757,-32728,-32678,-32609,-32521,-32412,-32285,-32137,-31971,-31785,-31580,-31356,-31113,-30852,-30571,
    -30273,-29956,-29621,-29268,-28898,-28510,-28105,-27683,-27245,-26790,-26319,-25832,-25329,-24811,-24279,-23731,
    -23170,-22594,-22005,-21403,-20787,-20159,-19519,-18868,-18204,-17530,-16846,-16151,-15446,-14732,-14010,-13279,
    -12539,-11793,-11039,-10278,-9512,-8739,-7962,-7179,-6393,-5602,-4808,-4011,-3212,-2410,-1608,-804,
];
