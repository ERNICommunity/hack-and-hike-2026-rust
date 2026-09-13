//! Drawing helpers for applications.
//!
//! Applications own their screens and navigation; this module provides a
//! [`Canvas`] to draw on, the colour palette, text helpers, a slider widget
//! and the `embedded-gui` glue the demo uses.

pub mod canvas;
pub mod common;
pub mod font;
pub mod gui;
pub mod styles;
pub mod theme;
pub mod widgets;

pub use canvas::Canvas;
