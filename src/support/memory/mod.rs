//! Memory setup and reporting.
//!
//! The ESP32-S3 has two kinds of RAM:
//!
//! - **Internal RAM**: fast, but small. The firmware gets two heaps of about
//!   72 KiB each, and every task stack lives here.
//! - **PSRAM**: an external 8 MiB chip, slower to access but plentiful.
//!
//! Large, long-lived buffers (canvases, camera frames, the log history)
//! belong in PSRAM through [`storage`]; everything else stays in internal
//! RAM. Do not put large arrays on the stack: a 320x240 frame is 150 KiB.

mod psram;
pub mod storage;

use esp_alloc::HEAP;
use log::info;

pub(crate) use psram::enable as enable_psram;

/// Log how much internal heap and PSRAM is in use, with `label` to tell
/// reports apart. Handy when chasing an allocation failure.
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
