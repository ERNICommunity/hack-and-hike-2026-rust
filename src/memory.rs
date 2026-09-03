//! Physical memory policy and heap instrumentation.
//!
//! The global allocator is internal-RAM only. PSRAM is a dedicated allocator
//! for explicit data-plane buffers. Heap usage is monitored continuously so
//! internal SRAM becomes an explicit runtime budget.

use embassy_time::{Duration, Instant};
use esp_alloc::{EspHeap, HEAP, HeapRegion, MemoryCapability};
use esp_hal::{
    peripherals::PSRAM,
    psram::{Psram, PsramConfig, PsramMode},
};

const PERIODIC_REPORT_INTERVAL: Duration = Duration::from_secs(10);
const INTERNAL_WARN_FREE_BYTES: usize = 32 * 1024;
const INTERNAL_CRITICAL_FREE_BYTES: usize = 16 * 1024;

static PSRAM_HEAP: EspHeap = EspHeap::empty();

pub fn enable_psram(psram_peripheral: PSRAM<'static>) {
    let config = PsramConfig {
        mode: PsramMode::QuadSpi,
        ..PsramConfig::default()
    };

    let psram = Psram::new(psram_peripheral, config);
    let (start, size) = psram.raw_parts();

    unsafe {
        PSRAM_HEAP.add_region(HeapRegion::new(
            start,
            size,
            MemoryCapability::External.into(),
        ));
    }
}

pub fn psram_heap() -> &'static EspHeap {
    &PSRAM_HEAP
}

#[derive(Clone, Copy, Debug)]
pub struct StackSnapshot {
    /// Linker-defined usable CPU0 stack span, from the guard word to stack top.
    pub size: usize,
    /// Current distance from SP to the hardware-monitored guard word.
    pub headroom: usize,
}

/// Read the CPU0 stack range exported by ESP-HAL's linker script.
///
/// Use ESP-HAL's re-exported `xtensa_lx::get_stack_pointer()` instead of
/// application-level inline assembly. The Xtensa assembly is compiled inside
/// the architecture support crate, so this remains usable from our stable
/// application crate.
///
/// `_stack_end_cpu0` and `_stack_start_cpu0` are the exact bounds ESP-HAL uses
/// in its own stack-pointer range check.
pub fn cpu0_stack_snapshot() -> StackSnapshot {
    unsafe extern "C" {
        static _stack_end_cpu0: u32;
        static _stack_start_cpu0: u32;
    }

    let sp = esp_hal::xtensa_lx::get_stack_pointer() as usize;
    let bottom = (&raw const _stack_end_cpu0) as usize;
    let top = (&raw const _stack_start_cpu0) as usize;

    StackSnapshot {
        size: top.saturating_sub(bottom),
        headroom: sp.saturating_sub(bottom),
    }
}

#[derive(Clone, Copy, Debug)]
pub struct HeapSnapshot {
    pub internal_size: usize,
    pub internal_used: usize,
    pub internal_free: usize,
    pub internal_peak_used: usize,
    pub internal_total_allocated: u64,
    pub internal_total_freed: u64,
    pub psram_size: usize,
    pub psram_used: usize,
    pub psram_free: usize,
    pub psram_peak_used: usize,
    pub cpu0_stack_size: usize,
    pub cpu0_stack_headroom: usize,
}

impl HeapSnapshot {
    pub fn capture() -> Self {
        let internal = HEAP.stats();
        let psram = PSRAM_HEAP.stats();
        let stack = cpu0_stack_snapshot();

        Self {
            internal_size: internal.size,
            internal_used: internal.current_usage,
            internal_free: internal.size.saturating_sub(internal.current_usage),
            internal_peak_used: internal.max_usage,
            internal_total_allocated: internal.total_allocated,
            internal_total_freed: internal.total_freed,
            psram_size: psram.size,
            psram_used: psram.current_usage,
            psram_free: psram.size.saturating_sub(psram.current_usage),
            psram_peak_used: psram.max_usage,
            cpu0_stack_size: stack.size,
            cpu0_stack_headroom: stack.headroom,
        }
    }
}

pub fn report(label: &str) {
    let s = HeapSnapshot::capture();
    ::log::info!(
        "MEM [{}] int={}/{} KiB free={} KiB peak={} KiB | psram={}/{} KiB free={} KiB peak={} KiB | cpu0-stack={} KiB headroom={} KiB",
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
    );
}

#[derive(Clone, Copy, Debug)]
pub struct NavigationProbe {
    target_view: i32,
    before: HeapSnapshot,
}

/// Long-lived internal-memory monitor.
///
/// It tracks the lowest observed free SRAM, periodically emits current/peak
/// usage, warns on thresholds, and verifies whether a navigation redraw caused
/// allocator activity after UI prewarming.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HeapPressure {
    Normal,
    Low,
    Critical,
}

pub struct HeapMonitor {
    min_internal_free: usize,
    min_cpu0_stack_headroom: usize,
    last_periodic_report: Instant,
    pending_navigation: Option<NavigationProbe>,
    pressure: HeapPressure,
}

impl HeapMonitor {
    pub fn new(now: Instant) -> Self {
        let snapshot = HeapSnapshot::capture();
        Self {
            min_internal_free: snapshot.internal_free,
            min_cpu0_stack_headroom: snapshot.cpu0_stack_headroom,
            last_periodic_report: now,
            pending_navigation: None,
            pressure: pressure_for(snapshot.internal_free),
        }
    }

    pub fn checkpoint(&mut self, label: &str) {
        let snapshot = HeapSnapshot::capture();
        self.observe(snapshot);
        report(label);
    }

    pub fn begin_navigation(&mut self, target_view: i32) {
        self.pending_navigation = Some(NavigationProbe {
            target_view,
            before: HeapSnapshot::capture(),
        });
    }

    pub fn end_navigation(&mut self) {
        let Some(probe) = self.pending_navigation.take() else {
            return;
        };

        let after = HeapSnapshot::capture();
        self.observe(after);

        let allocated = after
            .internal_total_allocated
            .saturating_sub(probe.before.internal_total_allocated);
        let freed = after
            .internal_total_freed
            .saturating_sub(probe.before.internal_total_freed);
        let live_delta = after.internal_used as isize - probe.before.internal_used as isize;

        if allocated != 0 || freed != 0 || live_delta != 0 {
            ::log::warn!(
                "Navigation view={} touched internal heap: alloc={} B free={} B live_delta={} B",
                probe.target_view,
                allocated,
                freed,
                live_delta,
            );
        }
    }

    pub fn poll(&mut self, now: Instant) {
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

        if now - self.last_periodic_report >= PERIODIC_REPORT_INTERVAL {
            self.last_periodic_report = now;
            ::log::info!(
                "MEM runtime: internal free={} KiB min={} KiB peak={} KiB alloc={} KiB freed={} KiB | PSRAM free={} KiB | CPU0 stack headroom={} KiB min-observed={} KiB",
                snapshot.internal_free / 1024,
                self.min_internal_free / 1024,
                snapshot.internal_peak_used / 1024,
                snapshot.internal_total_allocated / 1024,
                snapshot.internal_total_freed / 1024,
                snapshot.psram_free / 1024,
                snapshot.cpu0_stack_headroom / 1024,
                self.min_cpu0_stack_headroom / 1024,
            );
        }
    }

    fn observe(&mut self, snapshot: HeapSnapshot) {
        self.min_internal_free = self.min_internal_free.min(snapshot.internal_free);
        self.min_cpu0_stack_headroom = self
            .min_cpu0_stack_headroom
            .min(snapshot.cpu0_stack_headroom);
    }
}

fn pressure_for(internal_free: usize) -> HeapPressure {
    if internal_free <= INTERNAL_CRITICAL_FREE_BYTES {
        HeapPressure::Critical
    } else if internal_free <= INTERNAL_WARN_FREE_BYTES {
        HeapPressure::Low
    } else {
        HeapPressure::Normal
    }
}
