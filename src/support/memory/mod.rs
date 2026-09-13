//! Memory setup and reporting.
//!
//! The ESP32-S3 has a small internal heap and a large external PSRAM. Large,
//! long-lived buffers (framebuffers, camera frames, audio history) belong in
//! PSRAM through [`storage`]; everything else stays in internal RAM.

mod psram;
pub mod storage;

use esp_alloc::HEAP;
use log::info;

pub(crate) use psram::enable as enable_psram;

/// Log how much internal heap and PSRAM is in use.
pub fn report(label: &str) {
    let internal = HEAP.stats();
    let external = psram::heap().stats();
    info!(
        "MEM [{}] internal={}/{} KiB (peak {} KiB) | psram={}/{} KiB (peak {} KiB)",
        label,
        internal.current_usage / 1024,
        internal.size / 1024,
        internal.max_usage / 1024,
        external.current_usage / 1024,
        external.size / 1024,
        external.max_usage / 1024,
    );
}
