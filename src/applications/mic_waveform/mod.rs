//! Stock microphone waveform application.
//!
//! The application owns waveform transformation, refresh policy, and rendering.
//! It consumes only the semantic microphone PCM capability plus the display
//! surface supplied by the common UI shell.

mod model;
mod view;

pub(crate) use model::{MAX_AMPLITUDE_PIXELS, Model, POINTS, WaveformFrame};
pub(crate) use view::View;
