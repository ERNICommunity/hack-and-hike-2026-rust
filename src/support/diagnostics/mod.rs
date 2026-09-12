//! Low-overhead runtime counters shared by CPU0 and CPU1.
//!
//! High-frequency paths increment atomics instead of logging directly. CPU0
//! periodically snapshots the counters alongside memory/stack diagnostics.

use core::sync::atomic::{AtomicU32, Ordering};

#[cfg(feature = "touch")]
static TOUCH_READ_ERRORS: AtomicU32 = AtomicU32::new(0);
#[cfg(feature = "touch")]
static TOUCH_EDGE_DROPS: AtomicU32 = AtomicU32::new(0);
#[cfg(any(feature = "mic", feature = "speaker"))]
static AUDIO_CAPTURE_ERRORS: AtomicU32 = AtomicU32::new(0);
#[cfg(any(feature = "mic", feature = "speaker"))]
static AUDIO_PLAYBACK_ERRORS: AtomicU32 = AtomicU32::new(0);
#[cfg(any(feature = "mic", feature = "speaker"))]
static AUDIO_FULL_DRAINS: AtomicU32 = AtomicU32::new(0);
#[cfg(feature = "network")]
static NETWORK_INIT_ERRORS: AtomicU32 = AtomicU32::new(0);
#[cfg(feature = "network")]
static NETWORK_TX_PACKETS: AtomicU32 = AtomicU32::new(0);
#[cfg(feature = "network")]
static NETWORK_RX_PACKETS: AtomicU32 = AtomicU32::new(0);
#[cfg(feature = "network")]
static NETWORK_TX_ERRORS: AtomicU32 = AtomicU32::new(0);
#[cfg(feature = "network")]
static NETWORK_RX_INVALID: AtomicU32 = AtomicU32::new(0);
#[cfg(feature = "network")]
static NETWORK_PEER_EVICTIONS: AtomicU32 = AtomicU32::new(0);

#[cfg(feature = "app-stock")]
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct RuntimeCounters {
    pub(crate) touch_read_errors: u32,
    pub(crate) touch_edge_drops: u32,
    pub(crate) audio_capture_errors: u32,
    pub(crate) audio_playback_errors: u32,
    /// Number of times one DMA pop drained the complete circular buffer.
    ///
    /// This is a saturation warning, not proof that hardware overran.
    pub(crate) audio_full_drains: u32,
    pub(crate) network_init_errors: u32,
    pub(crate) network_tx_packets: u32,
    pub(crate) network_rx_packets: u32,
    pub(crate) network_tx_errors: u32,
    pub(crate) network_rx_invalid: u32,
    pub(crate) network_peer_evictions: u32,
}

#[cfg(feature = "touch")]
pub(crate) fn record_touch_read_error() {
    TOUCH_READ_ERRORS.fetch_add(1, Ordering::Relaxed);
}

#[cfg(feature = "touch")]
pub(crate) fn record_touch_edge_drop() {
    TOUCH_EDGE_DROPS.fetch_add(1, Ordering::Relaxed);
}

#[cfg(any(feature = "mic", feature = "speaker"))]
pub(crate) fn record_audio_capture_error() {
    AUDIO_CAPTURE_ERRORS.fetch_add(1, Ordering::Relaxed);
}

#[cfg(any(feature = "mic", feature = "speaker"))]
pub(crate) fn record_audio_playback_error() {
    AUDIO_PLAYBACK_ERRORS.fetch_add(1, Ordering::Relaxed);
}

#[cfg(any(feature = "mic", feature = "speaker"))]
pub(crate) fn record_audio_full_drain() {
    AUDIO_FULL_DRAINS.fetch_add(1, Ordering::Relaxed);
}

#[cfg(feature = "network")]
pub(crate) fn record_network_init_error() {
    NETWORK_INIT_ERRORS.fetch_add(1, Ordering::Relaxed);
}

#[cfg(feature = "network")]
pub(crate) fn record_network_tx_packet() {
    NETWORK_TX_PACKETS.fetch_add(1, Ordering::Relaxed);
}

#[cfg(feature = "network")]
pub(crate) fn record_network_rx_packet() {
    NETWORK_RX_PACKETS.fetch_add(1, Ordering::Relaxed);
}

#[cfg(feature = "network")]
pub(crate) fn record_network_tx_error() {
    NETWORK_TX_ERRORS.fetch_add(1, Ordering::Relaxed);
}

#[cfg(feature = "network")]
pub(crate) fn record_network_rx_invalid() {
    NETWORK_RX_INVALID.fetch_add(1, Ordering::Relaxed);
}

#[cfg(feature = "network")]
pub(crate) fn record_network_peer_eviction() {
    NETWORK_PEER_EVICTIONS.fetch_add(1, Ordering::Relaxed);
}

#[cfg(feature = "app-stock")]
pub(crate) fn snapshot() -> RuntimeCounters {
    RuntimeCounters {
        touch_read_errors: TOUCH_READ_ERRORS.load(Ordering::Relaxed),
        touch_edge_drops: TOUCH_EDGE_DROPS.load(Ordering::Relaxed),
        audio_capture_errors: AUDIO_CAPTURE_ERRORS.load(Ordering::Relaxed),
        audio_playback_errors: AUDIO_PLAYBACK_ERRORS.load(Ordering::Relaxed),
        audio_full_drains: AUDIO_FULL_DRAINS.load(Ordering::Relaxed),
        network_init_errors: NETWORK_INIT_ERRORS.load(Ordering::Relaxed),
        network_tx_packets: NETWORK_TX_PACKETS.load(Ordering::Relaxed),
        network_rx_packets: NETWORK_RX_PACKETS.load(Ordering::Relaxed),
        network_tx_errors: NETWORK_TX_ERRORS.load(Ordering::Relaxed),
        network_rx_invalid: NETWORK_RX_INVALID.load(Ordering::Relaxed),
        network_peer_evictions: NETWORK_PEER_EVICTIONS.load(Ordering::Relaxed),
    }
}
