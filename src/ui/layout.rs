//! Authoritative presentation geometry.
//!
//! Physical panel dimensions come from `display`; all UI-specific geometry is
//! defined here so touch hit-testing, framebuffer composition, and partial LCD
//! updates cannot silently drift apart.

use crate::display::{self, Region};

pub const NAV_WIDTH: usize = 44;
pub const NAV_BUTTON_HEIGHT: usize = 48;

pub const CONTENT_X: usize = NAV_WIDTH;
pub const CONTENT_WIDTH: usize = display::WIDTH - CONTENT_X;
pub const CONTENT_HEIGHT: usize = display::HEIGHT;

pub const NAV_REGION: Region = Region::new(0, 0, NAV_WIDTH, display::HEIGHT);
pub const CONTENT_REGION: Region = Region::new(CONTENT_X, 0, CONTENT_WIDTH, CONTENT_HEIGHT);

pub const WAVEFORM_X: usize = 10;
pub const WAVEFORM_CANVAS_WIDTH: usize = 256;
pub const WAVEFORM_CANVAS_HEIGHT: usize = 90;
pub const WAVEFORM_CENTER_Y: i32 = (WAVEFORM_CANVAS_HEIGHT as i32) / 2;
pub const LEFT_WAVEFORM_Y: usize = 22;
pub const RIGHT_WAVEFORM_Y: usize = 140;

pub const LEFT_WAVEFORM_REGION: Region = Region::new(
    CONTENT_X + WAVEFORM_X,
    LEFT_WAVEFORM_Y,
    WAVEFORM_CANVAS_WIDTH,
    WAVEFORM_CANVAS_HEIGHT,
);
pub const RIGHT_WAVEFORM_REGION: Region = Region::new(
    CONTENT_X + WAVEFORM_X,
    RIGHT_WAVEFORM_Y,
    WAVEFORM_CANVAS_WIDTH,
    WAVEFORM_CANVAS_HEIGHT,
);

pub const TEXT_VISIBLE_LINES: usize = 23;
pub const TEXT_TOP: i32 = 4;
pub const TEXT_LINE_HEIGHT: i32 = 10;

const _: () = assert!(NAV_WIDTH < display::WIDTH);
const _: () = assert!(WAVEFORM_X + WAVEFORM_CANVAS_WIDTH <= CONTENT_WIDTH);
const _: () = assert!(LEFT_WAVEFORM_Y + WAVEFORM_CANVAS_HEIGHT <= CONTENT_HEIGHT);
const _: () = assert!(RIGHT_WAVEFORM_Y + WAVEFORM_CANVAS_HEIGHT <= CONTENT_HEIGHT);
