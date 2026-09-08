//! Physical memory policy and heap instrumentation.
//!
//! The global allocator is internal-RAM only. PSRAM is a dedicated allocator
//! for explicit data-plane and framebuffer storage. Heap usage is monitored
//! continuously so internal SRAM remains an explicit runtime budget.

use core::sync::atomic::{AtomicUsize, Ordering};

use embassy_time::{Duration, Instant, Timer};
use esp_alloc::{EspHeap, HEAP, HeapRegion, MemoryCapability};
use esp_hal::{
    peripherals::PSRAM,
    psram::{Psram, PsramConfig, PsramMode},
    system::Stack,
};

use crate::diagnostics;

const PERIODIC_REPORT_INTERVAL: Duration = Duration::from_secs(10);
const INTERNAL_WARN_FREE_BYTES: usize = 32 * 1024;
const INTERNAL_CRITICAL_FREE_BYTES: usize = 16 * 1024;

const STACK_WATERMARK_PATTERN: u32 = 0xA5A5_A5A5;
// ESP-RTOS currently places its guard near the bottom of each main-task stack.
// Reserve substantially more than that implementation detail so painting never
// touches the guard/control area.
const STACK_WATERMARK_RESERVED_BYTES: usize = 256;
// Never paint right up to the live SP; leave room for this function and its
// caller to return without touching freshly painted memory.
const STACK_WATERMARK_SAFETY_BYTES: usize = 256;
const STACK_WORD_BYTES: usize = core::mem::size_of::<u32>();

static PSRAM_HEAP: EspHeap = EspHeap::empty();

static CPU0_WATERMARK_START: AtomicUsize = AtomicUsize::new(0);
static CPU0_WATERMARK_END: AtomicUsize = AtomicUsize::new(0);
static CPU0_MIN_HEADROOM: AtomicUsize = AtomicUsize::new(0);

static CPU1_STACK_BOTTOM: AtomicUsize = AtomicUsize::new(0);
static CPU1_STACK_TOP: AtomicUsize = AtomicUsize::new(0);
static CPU1_WATERMARK_START: AtomicUsize = AtomicUsize::new(0);
static CPU1_WATERMARK_END: AtomicUsize = AtomicUsize::new(0);
static CPU1_MIN_HEADROOM: AtomicUsize = AtomicUsize::new(0);

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

fn cpu0_stack_bounds() -> (usize, usize) {
    unsafe extern "C" {
        static _stack_end_cpu0: u32;
        static _stack_start_cpu0: u32;
    }

    (
        (&raw const _stack_end_cpu0) as usize,
        (&raw const _stack_start_cpu0) as usize,
    )
}

fn align_up(value: usize, alignment: usize) -> usize {
    (value + alignment - 1) & !(alignment - 1)
}

fn align_down(value: usize, alignment: usize) -> usize {
    value & !(alignment - 1)
}

unsafe fn paint_stack_range(start: usize, end: usize) {
    let mut address = start;
    while address + STACK_WORD_BYTES <= end {
        unsafe {
            (address as *mut u32).write_volatile(STACK_WATERMARK_PATTERN);
        }
        address += STACK_WORD_BYTES;
    }
}

fn scan_stack_watermark(bottom: usize, start: usize, end: usize) -> Option<usize> {
    if start == 0 || end <= start {
        return None;
    }

    let mut address = start;
    while address + STACK_WORD_BYTES <= end {
        let value = unsafe { (address as *const u32).read_volatile() };
        if value != STACK_WATERMARK_PATTERN {
            return Some(address.saturating_sub(bottom));
        }
        address += STACK_WORD_BYTES;
    }

    Some(end.saturating_sub(bottom))
}

fn paint_live_stack(
    bottom: usize,
    top: usize,
    watermark_start: &AtomicUsize,
    watermark_end: &AtomicUsize,
    min_headroom: &AtomicUsize,
) {
    let sp = esp_hal::xtensa_lx::get_stack_pointer() as usize;
    let start = align_up(
        bottom.saturating_add(STACK_WATERMARK_RESERVED_BYTES),
        STACK_WORD_BYTES,
    );
    let end = align_down(
        sp.saturating_sub(STACK_WATERMARK_SAFETY_BYTES).min(top),
        STACK_WORD_BYTES,
    );

    if end <= start {
        return;
    }

    unsafe {
        paint_stack_range(start, end);
    }

    watermark_start.store(start, Ordering::Release);
    watermark_end.store(end, Ordering::Release);
    min_headroom.store(end.saturating_sub(bottom), Ordering::Release);
}

/// Paint the currently-unused part of the CPU0 main stack once during startup.
///
/// Later scans recover the deepest stack use even if the stack has already
/// unwound by the time diagnostics run.
pub fn init_cpu0_stack_watermark() {
    let (bottom, top) = cpu0_stack_bounds();
    paint_live_stack(
        bottom,
        top,
        &CPU0_WATERMARK_START,
        &CPU0_WATERMARK_END,
        &CPU0_MIN_HEADROOM,
    );
}

/// Record the address range of the statically allocated CPU1 stack before it is
/// handed to ESP-RTOS. Painting happens from CPU1 after its scheduler is live.
pub fn register_cpu1_stack<const SIZE: usize>(stack: &mut Stack<SIZE>) {
    CPU1_STACK_BOTTOM.store(stack.bottom() as usize, Ordering::Release);
    CPU1_STACK_TOP.store(stack.top() as usize, Ordering::Release);
}

/// Paint CPU1's currently-unused stack. Call this from the CPU1 entry closure.
pub fn init_cpu1_stack_watermark() {
    let bottom = CPU1_STACK_BOTTOM.load(Ordering::Acquire);
    let top = CPU1_STACK_TOP.load(Ordering::Acquire);
    if bottom == 0 || top <= bottom {
        return;
    }

    paint_live_stack(
        bottom,
        top,
        &CPU1_WATERMARK_START,
        &CPU1_WATERMARK_END,
        &CPU1_MIN_HEADROOM,
    );
}

fn update_cpu0_stack_watermark() {
    let (bottom, _) = cpu0_stack_bounds();
    if let Some(headroom) = scan_stack_watermark(
        bottom,
        CPU0_WATERMARK_START.load(Ordering::Acquire),
        CPU0_WATERMARK_END.load(Ordering::Acquire),
    ) {
        CPU0_MIN_HEADROOM.store(headroom, Ordering::Release);
    }
}

fn update_cpu1_stack_watermark() {
    let bottom = CPU1_STACK_BOTTOM.load(Ordering::Acquire);
    if let Some(headroom) = scan_stack_watermark(
        bottom,
        CPU1_WATERMARK_START.load(Ordering::Acquire),
        CPU1_WATERMARK_END.load(Ordering::Acquire),
    ) {
        CPU1_MIN_HEADROOM.store(headroom, Ordering::Release);
    }
}

/// CPU1 performs its own watermark scan so CPU0 never reads memory while CPU1
/// may be actively using that stack.
#[embassy_executor::task]
pub async fn cpu1_stack_monitor_task() {
    loop {
        update_cpu1_stack_watermark();
        Timer::after(PERIODIC_REPORT_INTERVAL).await;
    }
}

#[derive(Clone, Copy, Debug)]
pub struct StackSnapshot {
    /// Linker-defined usable CPU0 stack span, from the guard word to stack top.
    pub size: usize,
    /// Current distance from SP to the bottom of the linker-defined stack.
    pub headroom: usize,
}

/// Sample CPU0's current stack pointer. This is useful for live context, while
/// the painted watermark records the true deepest use since startup.
pub fn cpu0_stack_snapshot() -> StackSnapshot {
    let sp = esp_hal::xtensa_lx::get_stack_pointer() as usize;
    let (bottom, top) = cpu0_stack_bounds();

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
    pub cpu0_stack_min_headroom: usize,
    pub cpu1_stack_size: usize,
    pub cpu1_stack_min_headroom: usize,
}

impl HeapSnapshot {
    pub fn capture() -> Self {
        let internal = HEAP.stats();
        let psram = PSRAM_HEAP.stats();
        let stack = cpu0_stack_snapshot();
        let cpu1_bottom = CPU1_STACK_BOTTOM.load(Ordering::Acquire);
        let cpu1_top = CPU1_STACK_TOP.load(Ordering::Acquire);

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
            cpu0_stack_min_headroom: CPU0_MIN_HEADROOM.load(Ordering::Acquire),
            cpu1_stack_size: cpu1_top.saturating_sub(cpu1_bottom),
            cpu1_stack_min_headroom: CPU1_MIN_HEADROOM.load(Ordering::Acquire),
        }
    }
}

pub fn report(label: &str) {
    update_cpu0_stack_watermark();
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

/// Snapshot window used to detect allocator activity overlapping a named CPU0
/// operation.
///
/// The global allocator is shared with runtime/radio work, so a delta proves
/// overlap, not causality. This type intentionally has no presentation-specific
/// fields.
#[derive(Clone, Copy, Debug)]
struct HeapActivityProbe {
    label: &'static str,
    before: HeapSnapshot,
}

/// Long-lived internal-memory monitor.
///
/// It tracks the lowest observed free SRAM, periodically emits current/peak
/// usage, warns on pressure, and can report allocator activity that overlaps a
/// short named CPU0 operation. Activity probes are diagnostic correlation only:
/// another core may allocate during the same window.
pub struct HeapMonitor {
    min_internal_free: usize,
    last_periodic_report: Instant,
    last_periodic_allocated: u64,
    last_periodic_freed: u64,
    pending_activity: Option<HeapActivityProbe>,
    pressure: HeapPressure,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HeapPressure {
    Normal,
    Low,
    Critical,
}

impl HeapMonitor {
    pub fn new(now: Instant) -> Self {
        update_cpu0_stack_watermark();
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

    pub fn checkpoint(&mut self, label: &str) {
        update_cpu0_stack_watermark();
        let snapshot = HeapSnapshot::capture();
        self.observe(snapshot);
        self.last_periodic_allocated = snapshot.internal_total_allocated;
        self.last_periodic_freed = snapshot.internal_total_freed;
        report(label);
    }

    /// Begin a short correlation window for a statically named CPU0 operation.
    pub fn begin_activity(&mut self, label: &'static str) {
        self.pending_activity = Some(HeapActivityProbe {
            label,
            before: HeapSnapshot::capture(),
        });
    }

    /// End the current correlation window and report any global heap activity.
    pub fn end_activity(&mut self) {
        let Some(probe) = self.pending_activity.take() else {
            return;
        };

        update_cpu0_stack_watermark();
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
                "Internal heap activity overlapped {}: alloc={} B free={} B live_delta={} B",
                probe.label,
                allocated,
                freed,
                live_delta,
            );
        }
    }

    pub fn poll(&mut self, now: Instant) {
        let periodic_report = now - self.last_periodic_report >= PERIODIC_REPORT_INTERVAL;
        if periodic_report {
            update_cpu0_stack_watermark();
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

fn pressure_for(free_bytes: usize) -> HeapPressure {
    if free_bytes <= INTERNAL_CRITICAL_FREE_BYTES {
        HeapPressure::Critical
    } else if free_bytes <= INTERNAL_WARN_FREE_BYTES {
        HeapPressure::Low
    } else {
        HeapPressure::Normal
    }
}
