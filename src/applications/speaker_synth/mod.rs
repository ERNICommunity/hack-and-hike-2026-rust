//! Stock speaker synth application.
//!
//! This application owns melody/chime synthesis, tempo/pitch state and speaker
//! controls. It continuously generates PCM and feeds only the generic speaker
//! capability supplied by firmware composition.

mod chime;
mod melody;
mod model;
mod view;

pub(crate) use model::{Model, PitchSemitones, SpeakerDisplay, TempoBpm};
pub(crate) use view::View;

#[derive(Clone, Copy, Debug)]
pub(crate) enum Action {
    TogglePlayback,
    PlayOneShot,
    SetTempo(TempoBpm),
    SetPitch(PitchSemitones),
}
