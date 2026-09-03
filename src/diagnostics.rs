//! Low-overhead runtime counters shared by CPU0 and CPU1.
//!
//! High-frequency paths increment atomics instead of logging directly. CPU0
//! periodically snapshots the counters alongside memory/stack diagnostics.

use core::sync::atomic::{AtomicU32, Ordering};

static TOUCH_READ_ERRORS: AtomicU32 = AtomicU32::new(0);
static TOUCH_EDGE_DROPS: AtomicU32 = AtomicU32::new(0);
static AUDIO_CAPTURE_ERRORS: AtomicU32 = AtomicU32::new(0);
static AUDIO_FULL_DRAINS: AtomicU32 = AtomicU32::new(0);

#[derive(Clone, Copy, Debug, Default)]
pub struct RuntimeCounters {
    pub touch_read_errors: u32,
    pub touch_edge_drops: u32,
    pub audio_capture_errors: u32,
    /// Number of times one DMA pop drained the complete circular buffer.
    ///
    /// This is a saturation warning, not proof that hardware overran.
    pub audio_full_drains: u32,
}

pub fn record_touch_read_error() {
    TOUCH_READ_ERRORS.fetch_add(1, Ordering::Relaxed);
}

pub fn record_touch_edge_drop() {
    TOUCH_EDGE_DROPS.fetch_add(1, Ordering::Relaxed);
}

pub fn record_audio_capture_error() {
    AUDIO_CAPTURE_ERRORS.fetch_add(1, Ordering::Relaxed);
}

pub fn record_audio_full_drain() {
    AUDIO_FULL_DRAINS.fetch_add(1, Ordering::Relaxed);
}

pub fn snapshot() -> RuntimeCounters {
    RuntimeCounters {
        touch_read_errors: TOUCH_READ_ERRORS.load(Ordering::Relaxed),
        touch_edge_drops: TOUCH_EDGE_DROPS.load(Ordering::Relaxed),
        audio_capture_errors: AUDIO_CAPTURE_ERRORS.load(Ordering::Relaxed),
        audio_full_drains: AUDIO_FULL_DRAINS.load(Ordering::Relaxed),
    }
}
