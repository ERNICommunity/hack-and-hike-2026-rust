//! Synthesizer score derived from `assets/Untitled.mid`.
//!
//! Source MIDI: format 0, 384 ticks/quarter, 4/4, 120 BPM. The file contains
//! eight monophonic notes starting on quarter-note boundaries. Each note lasts
//! 96 ticks; the loop is explicitly extended to 3072 ticks so it is exactly
//! two 4/4 bars, including the final 288-tick rest.

use super::{PitchSemitones, TempoBpm, SAMPLE_RATE_HZ};

const TICKS_PER_BEAT: u32 = 384;
const LOOP_TICKS: u32 = 3_072;
const MIDI_MIN: i16 = 24;
const MIDI_MAX: i16 = 60;

#[derive(Clone, Copy)]
struct MidiNote {
    start_tick: u16,
    duration_ticks: u16,
    midi: u8,
    velocity: u8,
}

const NOTES: [MidiNote; 8] = [
    MidiNote { start_tick: 0, duration_ticks: 96, midi: 36, velocity: 50 },
    MidiNote { start_tick: 384, duration_ticks: 96, midi: 38, velocity: 50 },
    MidiNote { start_tick: 768, duration_ticks: 96, midi: 40, velocity: 50 },
    MidiNote { start_tick: 1152, duration_ticks: 96, midi: 41, velocity: 50 },
    MidiNote { start_tick: 1536, duration_ticks: 96, midi: 43, velocity: 50 },
    MidiNote { start_tick: 1920, duration_ticks: 96, midi: 45, velocity: 50 },
    MidiNote { start_tick: 2304, duration_ticks: 96, midi: 47, velocity: 50 },
    MidiNote { start_tick: 2688, duration_ticks: 96, midi: 48, velocity: 50 },
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

    pub(super) fn next_sample(
        &mut self,
        tempo: TempoBpm,
        pitch: PitchSemitones,
    ) -> i16 {
        let loop_q32 = u64::from(LOOP_TICKS) << 32;
        if self.tick_q32 >= loop_q32 {
            self.tick_q32 %= loop_q32;
            self.phase = 0;
            self.note_index = 0;
        }

        while self.note_index < NOTES.len() {
            let note = NOTES[self.note_index];
            let end_tick = u32::from(note.start_tick) + u32::from(note.duration_ticks);
            if self.tick_q32 < (u64::from(end_tick) << 32) {
                break;
            }
            self.note_index += 1;
            self.phase = 0;
        }

        let sample = if let Some(note) = NOTES.get(self.note_index).copied() {
            let start_q32 = u64::from(note.start_tick) << 32;
            if self.tick_q32 >= start_q32 {
                self.render_note(note, self.tick_q32 - start_q32, pitch)
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
        pitch: PitchSemitones,
    ) -> i16 {
        let midi = i16::from(note.midi) + i16::from(pitch.get());
        let step = phase_step(midi);

        let fundamental = i32::from(SINE_256[(self.phase >> 24) as usize]);
        let second_phase = self.phase.wrapping_mul(2);
        let second = i32::from(SINE_256[(second_phase >> 24) as usize]);
        self.phase = self.phase.wrapping_add(step);

        // Mostly sine, with a quiet second harmonic. This stays soft but helps
        // the small onboard speaker reproduce the MIDI's low notes.
        let wave = (fundamental * 4 + second) / 5;
        let envelope = envelope_q15(position_q32, note.duration_ticks);
        let amplitude = ((9_000 * i32::from(note.velocity)) / 64).clamp(2_000, 9_000);

        (((wave * amplitude) / 32_767) * envelope / 32_767) as i16
    }
}

fn tick_step_q32(tempo: TempoBpm) -> u64 {
    let numerator =
        u64::from(tempo.get()) * u64::from(TICKS_PER_BEAT) * (1u64 << 32);
    let denominator = 60 * u64::from(SAMPLE_RATE_HZ);
    (numerator / denominator).max(1)
}

fn envelope_q15(position_q32: u64, duration_ticks: u16) -> i32 {
    const ATTACK_TICKS: u64 = 8;
    const RELEASE_TICKS: u64 = 24;

    let duration_q32 = u64::from(duration_ticks) << 32;
    let attack_q32 = ATTACK_TICKS << 32;
    let release_q32 = RELEASE_TICKS << 32;

    let attack = ((position_q32.min(attack_q32) * 32_767) / attack_q32) as i32;
    let remaining_q32 = duration_q32.saturating_sub(position_q32);
    let release = ((remaining_q32.min(release_q32) * 32_767) / release_q32) as i32;
    attack.min(release).clamp(0, 32_767)
}

fn phase_step(midi: i16) -> u32 {
    const STEPS: [u32; 37] = [
        8778697, 9300706, 9853754, 10439689, 11060465, 11718155, 12414953,
        13153184, 13935313, 14763950, 15641860, 16571974, 17557394, 18601411,
        19707509, 20879378, 22120931, 23436310, 24829905, 26306368, 27870626,
        29527900, 31283720, 33143947, 35114789, 37202823, 39415018, 41758757,
        44241862, 46872620, 49659811, 52612737, 55741253, 59055800, 62567441,
        66287895, 70229578,
    ];
    STEPS[(midi.clamp(MIDI_MIN, MIDI_MAX) - MIDI_MIN) as usize]
}

const SINE_256: [i16; 256] = [
    0, 804, 1608, 2410, 3212, 4011, 4808, 5602, 6393, 7179, 7962, 8739, 9512, 10278, 11039, 11793,
    12539, 13279, 14010, 14732, 15446, 16151, 16846, 17530, 18204, 18868, 19519, 20159, 20787, 21403, 22005, 22594,
    23170, 23731, 24279, 24811, 25329, 25832, 26319, 26790, 27245, 27683, 28105, 28510, 28898, 29268, 29621, 29956,
    30273, 30571, 30852, 31113, 31356, 31580, 31785, 31971, 32137, 32285, 32412, 32521, 32609, 32678, 32728, 32757,
    32767, 32757, 32728, 32678, 32609, 32521, 32412, 32285, 32137, 31971, 31785, 31580, 31356, 31113, 30852, 30571,
    30273, 29956, 29621, 29268, 28898, 28510, 28105, 27683, 27245, 26790, 26319, 25832, 25329, 24811, 24279, 23731,
    23170, 22594, 22005, 21403, 20787, 20159, 19519, 18868, 18204, 17530, 16846, 16151, 15446, 14732, 14010, 13279,
    12539, 11793, 11039, 10278, 9512, 8739, 7962, 7179, 6393, 5602, 4808, 4011, 3212, 2410, 1608, 804,
    0, -804, -1608, -2410, -3212, -4011, -4808, -5602, -6393, -7179, -7962, -8739, -9512, -10278, -11039, -11793,
    -12539, -13279, -14010, -14732, -15446, -16151, -16846, -17530, -18204, -18868, -19519, -20159, -20787, -21403, -22005, -22594,
    -23170, -23731, -24279, -24811, -25329, -25832, -26319, -26790, -27245, -27683, -28105, -28510, -28898, -29268, -29621, -29956,
    -30273, -30571, -30852, -31113, -31356, -31580, -31785, -31971, -32137, -32285, -32412, -32521, -32609, -32678, -32728, -32757,
    -32767, -32757, -32728, -32678, -32609, -32521, -32412, -32285, -32137, -31971, -31785, -31580, -31356, -31113, -30852, -30571,
    -30273, -29956, -29621, -29268, -28898, -28510, -28105, -27683, -27245, -26790, -26319, -25832, -25329, -24811, -24279, -23731,
    -23170, -22594, -22005, -21403, -20787, -20159, -19519, -18868, -18204, -17530, -16846, -16151, -15446, -14732, -14010, -13279,
    -12539, -11793, -11039, -10278, -9512, -8739, -7962, -7179, -6393, -5602, -4808, -4011, -3212, -2410, -1608, -804,
];
