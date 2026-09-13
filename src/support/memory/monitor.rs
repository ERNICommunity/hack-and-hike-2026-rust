//! Heap snapshots, pressure policy, and runtime memory diagnostics.

#[cfg(feature = "app-demo")]
use embassy_time::{Duration, Instant};
use esp_alloc::HEAP;

#[cfg(feature = "app-demo")]
use crate::support::diagnostics;

use super::{psram, stack};

#[cfg(feature = "app-demo")]
const PERIODIC_REPORT_INTERVAL: Duration = Duration::from_secs(10);
#[cfg(feature = "app-demo")]
const INTERNAL_WARN_FREE_BYTES: usize = 32 * 1024;
#[cfg(feature = "app-demo")]
const INTERNAL_CRITICAL_FREE_BYTES: usize = 16 * 1024;

#[derive(Clone, Copy, Debug)]
struct HeapSnapshot {
    internal_size: usize,
    internal_used: usize,
    internal_free: usize,
    internal_peak_used: usize,
    #[cfg(feature = "app-demo")]
    internal_total_allocated: u64,
    #[cfg(feature = "app-demo")]
    internal_total_freed: u64,
    psram_size: usize,
    psram_used: usize,
    psram_free: usize,
    psram_peak_used: usize,
    cpu0_stack_size: usize,
    cpu0_stack_headroom: usize,
    cpu0_stack_min_headroom: usize,
    cpu1_stack_size: usize,
    cpu1_stack_min_headroom: usize,
}

impl HeapSnapshot {
    fn capture() -> Self {
        let internal = HEAP.stats();
        let external = psram::heap().stats();
        let cpu0 = stack::cpu0_snapshot();

        Self {
            internal_size: internal.size,
            internal_used: internal.current_usage,
            internal_free: internal.size.saturating_sub(internal.current_usage),
            internal_peak_used: internal.max_usage,
            #[cfg(feature = "app-demo")]
            internal_total_allocated: internal.total_allocated,
            #[cfg(feature = "app-demo")]
            internal_total_freed: internal.total_freed,
            psram_size: external.size,
            psram_used: external.current_usage,
            psram_free: external.size.saturating_sub(external.current_usage),
            psram_peak_used: external.max_usage,
            cpu0_stack_size: cpu0.size,
            cpu0_stack_headroom: cpu0.headroom,
            cpu0_stack_min_headroom: stack::cpu0_min_headroom(),
            cpu1_stack_size: stack::cpu1_size(),
            cpu1_stack_min_headroom: stack::cpu1_min_headroom(),
        }
    }
}

pub(crate) fn report(label: &str) {
    stack::update_cpu0_stack_watermark();
    let s = HeapSnapshot::capture();
    ::log::info!(
        "MEM [{}] int={}/{} KiB free={} KiB peak={} KiB | psram={}/{} KiB free={} KiB peak={} KiB | cpu0-stack={} KiB current={} KiB min={} KiB | cpu1-stack={} KiB min={} KiB",
        label,
        s.internal_used / 1024,
        s.internal_size / 1024,
        s.internal_free / 1024,
        s.internal_peak_used / 1024,
        s.psram_used / 1024,
        s.psram_size / 1024,
        s.psram_free / 1024,
        s.psram_peak_used / 1024,
        s.cpu0_stack_size / 1024,
        s.cpu0_stack_headroom / 1024,
        s.cpu0_stack_min_headroom / 1024,
        s.cpu1_stack_size / 1024,
        s.cpu1_stack_min_headroom / 1024,
    );
}

/// Snapshot window used to detect allocator activity overlapping a named CPU0 operation.
#[cfg(feature = "app-demo")]
#[derive(Clone, Copy, Debug)]
struct HeapActivityProbe {
    label: &'static str,
    before: HeapSnapshot,
}

/// Long-lived internal-memory monitor.
#[cfg(feature = "app-demo")]
pub(crate) struct HeapMonitor {
    min_internal_free: usize,
    last_periodic_report: Instant,
    last_periodic_allocated: u64,
    last_periodic_freed: u64,
    pending_activity: Option<HeapActivityProbe>,
    pressure: HeapPressure,
}

#[cfg(feature = "app-demo")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HeapPressure {
    Normal,
    Low,
    Critical,
}

#[cfg(feature = "app-demo")]
impl HeapMonitor {
    pub(crate) fn new(now: Instant) -> Self {
        stack::update_cpu0_stack_watermark();
        let snapshot = HeapSnapshot::capture();
        Self {
            min_internal_free: snapshot.internal_free,
            last_periodic_report: now,
            last_periodic_allocated: snapshot.internal_total_allocated,
            last_periodic_freed: snapshot.internal_total_freed,
            pending_activity: None,
            pressure: pressure_for(snapshot.internal_free),
        }
    }

    pub(crate) fn checkpoint(&mut self, label: &str) {
        stack::update_cpu0_stack_watermark();
        let snapshot = HeapSnapshot::capture();
        self.observe(snapshot);
        self.last_periodic_allocated = snapshot.internal_total_allocated;
        self.last_periodic_freed = snapshot.internal_total_freed;
        report(label);
    }

    pub(crate) fn begin_activity(&mut self, label: &'static str) {
        self.pending_activity = Some(HeapActivityProbe {
            label,
            before: HeapSnapshot::capture(),
        });
    }

    pub(crate) fn end_activity(&mut self) {
        let Some(probe) = self.pending_activity.take() else {
            return;
        };

        stack::update_cpu0_stack_watermark();
        let after = HeapSnapshot::capture();
        self.observe(after);

        let allocated = after
            .internal_total_allocated
            .saturating_sub(probe.before.internal_total_allocated);
        let freed = after
            .internal_total_freed
            .saturating_sub(probe.before.internal_total_freed);
        let live_delta = after.internal_used as isize - probe.before.internal_used as isize;

        if live_delta > 0 {
            // A positive live delta means the operation retained internal heap;
            // unlike balanced scratch allocation, that can accumulate across
            // repeated navigation and is worth surfacing as a warning.
            ::log::warn!(
                "Internal heap retained growth during {}: alloc={} B free={} B live_delta={} B",
                probe.label,
                allocated,
                freed,
                live_delta,
            );
        } else if allocated != 0 || freed != 0 || live_delta != 0 {
            // embedded-gui legitimately uses short-lived allocator-backed
            // scratch storage while constructing some views. If live usage is
            // unchanged (or lower) when the operation completes, that activity
            // is not a leak or memory-pressure event.
            ::log::trace!(
                "Internal heap transient activity during {}: alloc={} B free={} B live_delta={} B",
                probe.label,
                allocated,
                freed,
                live_delta,
            );
        }
    }

    pub(crate) fn poll(&mut self, now: Instant) {
        let periodic_report = now - self.last_periodic_report >= PERIODIC_REPORT_INTERVAL;
        if periodic_report {
            stack::update_cpu0_stack_watermark();
        }

        let snapshot = HeapSnapshot::capture();
        self.observe(snapshot);

        let pressure = pressure_for(snapshot.internal_free);
        if pressure != self.pressure {
            self.pressure = pressure;
            match pressure {
                HeapPressure::Normal => ::log::info!(
                    "Internal heap recovered: free={} KiB",
                    snapshot.internal_free / 1024,
                ),
                HeapPressure::Low => ::log::warn!(
                    "Internal heap low: free={} KiB, peak={} KiB",
                    snapshot.internal_free / 1024,
                    snapshot.internal_peak_used / 1024,
                ),
                HeapPressure::Critical => ::log::error!(
                    "Internal heap CRITICAL: free={} KiB, peak={} KiB",
                    snapshot.internal_free / 1024,
                    snapshot.internal_peak_used / 1024,
                ),
            }
        }

        if periodic_report {
            self.last_periodic_report = now;
            let allocated_delta = snapshot
                .internal_total_allocated
                .saturating_sub(self.last_periodic_allocated);
            let freed_delta = snapshot
                .internal_total_freed
                .saturating_sub(self.last_periodic_freed);
            self.last_periodic_allocated = snapshot.internal_total_allocated;
            self.last_periodic_freed = snapshot.internal_total_freed;

            let counters = diagnostics::snapshot();
            ::log::info!(
                "MEM runtime: internal free={} KiB min={} KiB peak={} KiB alloc={} KiB freed={} KiB dalloc={} B dfree={} B | PSRAM free={} KiB | CPU0 stack current={} KiB min={} KiB | CPU1 stack={} KiB min={} KiB | diag touch-readerr={} touch-drops={} audio-errors={} audio-playback-errors={} audio-full-drains={} net-init={} net-tx={} net-rx={} net-txerr={} net-invalid={} net-evict={}",
                snapshot.internal_free / 1024,
                self.min_internal_free / 1024,
                snapshot.internal_peak_used / 1024,
                snapshot.internal_total_allocated / 1024,
                snapshot.internal_total_freed / 1024,
                allocated_delta,
                freed_delta,
                snapshot.psram_free / 1024,
                snapshot.cpu0_stack_headroom / 1024,
                snapshot.cpu0_stack_min_headroom / 1024,
                snapshot.cpu1_stack_size / 1024,
                snapshot.cpu1_stack_min_headroom / 1024,
                counters.touch_read_errors,
                counters.touch_edge_drops,
                counters.audio_capture_errors,
                counters.audio_playback_errors,
                counters.audio_full_drains,
                counters.network_init_errors,
                counters.network_tx_packets,
                counters.network_rx_packets,
                counters.network_tx_errors,
                counters.network_rx_invalid,
                counters.network_peer_evictions,
            );
        }
    }

    fn observe(&mut self, snapshot: HeapSnapshot) {
        self.min_internal_free = self.min_internal_free.min(snapshot.internal_free);
    }
}

#[cfg(feature = "app-demo")]
fn pressure_for(free_bytes: usize) -> HeapPressure {
    if free_bytes <= INTERNAL_CRITICAL_FREE_BYTES {
        HeapPressure::Critical
    } else if free_bytes <= INTERNAL_WARN_FREE_BYTES {
        HeapPressure::Low
    } else {
        HeapPressure::Normal
    }
}
