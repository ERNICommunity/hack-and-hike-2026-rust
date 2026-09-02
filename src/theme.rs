//! Build-generated theme constants.
//!
//! The authoritative values live in `theme.toml`. `build.rs` emits the Rust
//! constants included here and the matching Slint `Theme` global.

include!(concat!(env!("OUT_DIR"), "/theme_generated.rs"));
