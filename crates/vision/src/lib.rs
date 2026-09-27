//! Hardware-independent image processing of the Hack & Hike face
//! recognition app.
//!
//! The board's camera delivers 320x240 frames of big-endian RGB565 pixels.
//! This crate turns such frames into the small gray or RGB images that a
//! face detector and a face recognizer need, and judges whether a frame is
//! sharp enough to use. It is plain `no_std` Rust with no ESP32 dependency
//! and no allocation, so it compiles for your computer too and has ordinary
//! tests:
//!
//! ```text
//! ./scripts/test.sh -p hack-and-hike-vision
//! ```
//!
//! All image buffers are borrowed from the caller. The firmware owns the
//! memory (usually in PSRAM), and these functions only read and write it.
//! Nothing here puts an image on the stack: the task stacks are small.
//!
//! | Module | Contents | Used by |
//! | --- | --- | --- |
//! | [`pixel`] | one pixel at a time: RGB565 to RGB888 and to gray | the other modules, camera preview |
//! | [`image`] | image views over borrowed buffers, box downscaling of a camera frame | face detector input |
//! | [`warp`] | similarity transform, landmark fitting, bilinear warp (face alignment) | face recognizer input |
//! | [`quality`] | a blur score | frame selection |
//! | [`blob`] | the file format of model weights and test fixtures | the networks, the tests, `facekit` |
//! | [`nn`] | the neural-network layers in `f32`, and the two networks built from them | face detector, face recognizer |
//! | [`detect`] | the detector's input from a frame, and faces from its outputs | the application |
//! | [`gates`] | is the face framed, frontal and sharp enough to recognize? | the application |
//! | [`align`] | the recognizer's input: the face cut out by its landmarks | the application |
//! | [`gallery`] | enrolled people, and the decision "this is Alexander" or "unknown" | the application |
//!
//! Coordinates are always `(x, y)` with `x` along a row (0 at the left) and
//! `y` down the rows (0 at the top). An image with width `w` and `c` channels
//! stores pixel `(x, y)` at byte offset `(y * w + x) * c`.

#![no_std]
#![cfg_attr(target_arch = "xtensa", feature(asm_experimental_arch))]
#![deny(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::missing_docs_in_private_items)]

pub mod align;
pub mod blob;
pub mod detect;
pub mod gallery;
pub mod gates;
pub mod image;
pub mod nn;
pub mod pixel;
pub mod quality;
pub mod warp;
