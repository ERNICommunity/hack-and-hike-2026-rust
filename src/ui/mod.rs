//! Drawing helpers for applications.
//!
//! Applications own their screens and their navigation. This module gives
//! them the parts:
//!
//! - [`Canvas`]: an image to draw on with `embedded-graphics`. It sends only
//!   the changed pixels to the panel.
//! - [`theme`]: the colour palette of the project.
//! - [`common`]: fonts and text helpers.
//! - [`widgets`]: a slider that you control by touch.
//! - [`gui`], [`styles`], [`font`]: the connection to `embedded-gui`, a
//!   widget library. The demo uses it for layouts that are described in KDL
//!   files. Small applications do not need it.

pub mod canvas;
pub mod common;
pub mod font;
pub mod gui;
pub mod styles;
pub mod theme;
pub mod widgets;

pub use canvas::Canvas;
