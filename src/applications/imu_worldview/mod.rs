//! Stock IMU worldview application.
//!
//! This application owns its refresh/presentation state and worldview renderer.
//! It consumes only the semantic IMU capability plus the display surface supplied
//! by the common UI shell.

mod model;
mod view;

pub(crate) use model::{DisplayState, Model};
pub(crate) use view::View;
