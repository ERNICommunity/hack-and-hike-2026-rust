//! Drawing helpers for applications.
//!
//! Applications own their screens and navigation; this module provides the
//! pieces:
//!
//! - [`Canvas`]: an image to draw on with `embedded-graphics`, shown on the
//!   panel efficiently.
//! - [`theme`]: the project colour palette.
//! - [`common`]: fonts and text helpers.
//! - [`widgets`]: a touch slider.
//! - [`gui`], [`styles`], [`font`]: the glue for `embedded-gui`, which the
//!   demo uses for layouts described in KDL files. Small applications do not
//!   need it.

pub mod canvas;
pub mod common;
pub mod font;
pub mod gui;
pub mod styles;
pub mod theme;
pub mod widgets;

pub use canvas::Canvas;
