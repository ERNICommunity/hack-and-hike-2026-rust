//! Stock typed ESP-NOW messaging demonstration.
//!
//! The application owns its postcard schema, ping/pong behavior, presentation
//! state and view. The network capability remains schema-agnostic.

mod model;
mod view;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum DemoMessage {
    Ping { sequence: u32 },
    Pong { sequence: u32 },
}

pub(crate) use model::{DisplayState, Model};
pub(crate) use view::View;
