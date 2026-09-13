//! Synthesizer score transcribed from `assets/speaker_melody.mid`.
//!
//! Source MIDI: format 0, 384 ticks/quarter, 4/4, 120 BPM. The file contains
//! eight monophonic note-ons on quarter-note boundaries. The original note-offs
//! occur after 96 ticks; playback intentionally holds every note until the next
//! note-on (or the loop boundary) so each note is as long as possible without
//! overlapping the following note. Playback is transposed up two octaves.

use hack_and_hike::capabilities::audio::SAMPLE_RATE_HZ;

use super::{PitchSemitones, TempoBpm};

const TICKS_PER_BEAT: u32 = 384;
const LOOP_TICKS: u32 = 3_072;
const MIDI_MIN: i16 = 24;
const MIDI_MAX: i16 = 60;
const MIDI_OCTAVE_SHIFT: u32 = 2;
const MELODY_OUTPUT_PEAK: i32 = 5_000;
const SCORE_REFERENCE_VELOCITY: i32 = 50;

// Hardware testing shows the synthesized tone becomes clean between D5
// (~587 Hz) and E5 (~659 Hz). Do not feed a sustained fundamental below that
// boundary to the tiny speaker/amplifier. Instead synthesize two adjacent
// harmonics above it; their missing fundamental preserves the perceived note.
const DIRECT_FUNDAMENTAL_MIN_HZ: u32 = 620;
const DIRECT_FUNDAMENTAL_MIN_STEP: u32 =
    ((DIRECT_FUNDAMENTAL_MIN_HZ as u64 * (1u64 << 32)) / SAMPLE_RATE_HZ as u64) as u32;

#[derive(Clone, Copy)]
struct MidiNote {
    start_tick: u16,
    midi: u8,
    velocity: u8,
}

const NOTES: [MidiNote; 8] = [
    MidiNote {
        start_tick: 0,
        midi: 36,
        velocity: 50,
    },
    MidiNote {
        start_tick: 384,
        midi: 38,
        velocity: 50,
    },
    MidiNote {
        start_tick: 768,
        midi: 40,
        velocity: 50,
    },
    MidiNote {
        start_tick: 1152,
        midi: 41,
        velocity: 50,
    },
    MidiNote {
        start_tick: 1536,
        midi: 43,
        velocity: 50,
    },
    MidiNote {
        start_tick: 1920,
        midi: 45,
        velocity: 50,
    },
    MidiNote {
        start_tick: 2304,
        midi: 47,
        velocity: 50,
    },
    MidiNote {
        start_tick: 2688,
        midi: 48,
        velocity: 50,
    },
];

pub(super) struct MelodySynth {
    tick_q32: u64,
    phase: u32,
    note_index: usize,
}

impl MelodySynth {
    pub(super) const fn new() -> Self {
        Self {
            tick_q32: 0,
            phase: 0,
            note_index: 0,
        }
    }

    pub(super) fn restart(&mut self) {
        self.tick_q32 = 0;
        self.phase = 0;
        self.note_index = 0;
    }

    pub(super) fn next_sample(&mut self, tempo: TempoBpm, pitch: PitchSemitones) -> i16 {
        let loop_q32 = u64::from(LOOP_TICKS) << 32;
        if self.tick_q32 >= loop_q32 {
            self.tick_q32 %= loop_q32;
            self.phase = 0;
            self.note_index = 0;
        }

        while self.note_index < NOTES.len() {
            let end_tick = note_end_tick(self.note_index);
            if self.tick_q32 < (u64::from(end_tick) << 32) {
                break;
            }
            self.note_index += 1;
            self.phase = 0;
        }

        let sample = if let Some(note) = NOTES.get(self.note_index).copied() {
            let start_q32 = u64::from(note.start_tick) << 32;
            if self.tick_q32 >= start_q32 {
                let duration_ticks = note_end_tick(self.note_index) - u32::from(note.start_tick);
                self.render_note(note, self.tick_q32 - start_q32, duration_ticks, pitch)
            } else {
                0
            }
        } else {
            0
        };

        self.tick_q32 = self.tick_q32.saturating_add(tick_step_q32(tempo));
        sample
    }

    fn render_note(
        &mut self,
        note: MidiNote,
        position_q32: u64,
        duration_ticks: u32,
        pitch: PitchSemitones,
    ) -> i16 {
        let midi = i16::from(note.midi) + i16::from(pitch.get());
        // Doubling oscillator frequency per octave keeps the pitch slider
        // relative to the source notes while moving the whole melody +24 st.
        let step = phase_step(midi) << MIDI_OCTAVE_SHIFT;
        let wave = i32::from(speaker_safe_wave(self.phase, step));
        self.phase = self.phase.wrapping_add(step);

        // The supplied score uses velocity 50. Treat that as the maximum normal
        // melody level: lower velocities may attenuate, but no velocity can push
        // the synth above MELODY_OUTPUT_PEAK.
        let amplitude = ((MELODY_OUTPUT_PEAK * i32::from(note.velocity))
            / SCORE_REFERENCE_VELOCITY)
            .clamp(0, MELODY_OUTPUT_PEAK);
        let envelope = envelope_q15(position_q32, duration_ticks);

        (((wave * amplitude) / 32_767) * envelope / 32_767) as i16
    }
}

fn note_end_tick(index: usize) -> u32 {
    NOTES
        .get(index + 1)
        .map(|next| u32::from(next.start_tick))
        .unwrap_or(LOOP_TICKS)
}

fn tick_step_q32(tempo: TempoBpm) -> u64 {
    let numerator = u64::from(tempo.get()) * u64::from(TICKS_PER_BEAT) * (1u64 << 32);
    let denominator = 60 * u64::from(SAMPLE_RATE_HZ);
    (numerator / denominator).max(1)
}

fn envelope_q15(position_q32: u64, duration_ticks: u32) -> i32 {
    // Keep only a tiny edge taper to prevent clicks. There is no intentional
    // gap between notes: the release ends exactly where the next attack begins.
    const ATTACK_TICKS: u64 = 2;
    const RELEASE_TICKS: u64 = 2;

    let duration_q32 = u64::from(duration_ticks) << 32;
    let attack_q32 = ATTACK_TICKS << 32;
    let release_q32 = RELEASE_TICKS << 32;

    let attack = ((position_q32.min(attack_q32) * 32_767) / attack_q32) as i32;
    let remaining_q32 = duration_q32.saturating_sub(position_q32);
    let release = ((remaining_q32.min(release_q32) * 32_767) / release_q32) as i32;
    attack.min(release).clamp(0, 32_767)
}

/// Generate a speaker-safe tone without changing the perceived MIDI pitch.
///
/// Above the empirically clean boundary, emit the fundamental directly. Below
/// it, remove the problematic low fundamental and emit the lowest two adjacent
/// harmonics that are both above the boundary. Adjacent harmonics retain the
/// original period, so the ear reconstructs the missing fundamental while the
/// physical output no longer contains the frequency range that distorts.
fn speaker_safe_wave(phase: u32, fundamental_step: u32) -> i16 {
    if fundamental_step >= DIRECT_FUNDAMENTAL_MIN_STEP {
        return sine_wave(phase);
    }

    let step = u64::from(fundamental_step);
    let threshold = u64::from(DIRECT_FUNDAMENTAL_MIN_STEP);
    let first_harmonic = threshold.div_ceil(step) as u32;
    // The current pitch range requires at most the 5th harmonic. Keep a hard
    // bound in case the score range changes later.
    let first_harmonic = first_harmonic.clamp(2, 6);

    let first = i32::from(sine_wave(phase.wrapping_mul(first_harmonic)));
    let second = i32::from(sine_wave(
        phase.wrapping_mul(first_harmonic.saturating_add(1)),
    ));

    // A weighted average keeps the composite within Q15 without a limiter.
    ((first * 3 + second * 2) / 5) as i16
}

/// Interpolated Q15 sine lookup.
///
/// The old lookup discarded 24 phase bits. Linear interpolation keeps the DDS
/// phase resolution high enough that this synth does not add audible table-step
/// artifacts while still avoiding floating-point work in the application loop.
fn sine_wave(phase: u32) -> i16 {
    const QUARTER_TURN: u32 = 1 << 30;
    const SEGMENT_SHIFT: u32 = 24;
    const SEGMENT_MASK: u32 = (1 << SEGMENT_SHIFT) - 1;

    let quadrant = phase >> 30;
    let offset = phase & (QUARTER_TURN - 1);
    let quarter_phase = match quadrant {
        0 | 2 => offset,
        _ => QUARTER_TURN - offset,
    };

    let magnitude = if quarter_phase == QUARTER_TURN {
        32_767i32
    } else {
        let index = (quarter_phase >> SEGMENT_SHIFT) as usize;
        let fraction = i64::from(quarter_phase & SEGMENT_MASK);
        let a = i64::from(SINE_QUARTER_64[index]);
        let b = i64::from(SINE_QUARTER_64[index + 1]);
        (a + (((b - a) * fraction) >> SEGMENT_SHIFT)) as i32
    };

    if quadrant >= 2 {
        (-magnitude) as i16
    } else {
        magnitude as i16
    }
}

fn phase_step(midi: i16) -> u32 {
    const STEPS: [u32; 37] = [
        8778697, 9300706, 9853754, 10439689, 11060465, 11718155, 12414953, 13153184, 13935313,
        14763950, 15641860, 16571974, 17557394, 18601411, 19707509, 20879378, 22120931, 23436310,
        24829905, 26306368, 27870626, 29527900, 31283720, 33143947, 35114789, 37202823, 39415018,
        41758757, 44241862, 46872620, 49659811, 52612737, 55741253, 59055800, 62567441, 66287895,
        70229578,
    ];
    STEPS[(midi.clamp(MIDI_MIN, MIDI_MAX) - MIDI_MIN) as usize]
}

// One quarter-wave is sufficient because sine is symmetric. The 64 segments
// are linearly interpolated above, so this uses less flash and much less phase
// quantization than indexing the old 256-entry full-wave table directly.
const SINE_QUARTER_64: [i16; 65] = [
    0, 804, 1608, 2410, 3212, 4011, 4808, 5602, 6393, 7179, 7962, 8739, 9512, 10278, 11039, 11793,
    12539, 13279, 14010, 14732, 15446, 16151, 16846, 17530, 18204, 18868, 19519, 20159, 20787,
    21403, 22005, 22594, 23170, 23731, 24279, 24811, 25329, 25832, 26319, 26790, 27245, 27683,
    28105, 28510, 28898, 29268, 29621, 29956, 30273, 30571, 30852, 31113, 31356, 31580, 31785,
    31971, 32137, 32285, 32412, 32521, 32609, 32678, 32728, 32757, 32767,
];
