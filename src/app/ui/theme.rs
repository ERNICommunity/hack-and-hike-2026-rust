//! Build-generated RGB565 palette.
//!
//! The editable source is `theme.toml`. `build.rs` converts those RGB colors
//! into allocation-free RGB565 constants used by the presentation design specs.

include!(concat!(env!("OUT_DIR"), "/theme_generated.rs"));
