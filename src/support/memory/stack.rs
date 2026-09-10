//! CPU0/CPU1 stack watermarking and stack measurements.

use core::sync::atomic::{AtomicUsize, Ordering};

use embassy_time::{Duration, Timer};
use esp_hal::system::Stack;

const PERIODIC_REPORT_INTERVAL: Duration = Duration::from_secs(10);
const STACK_WATERMARK_PATTERN: u32 = 0xA5A5_A5A5;
// ESP-RTOS currently places its guard near the bottom of each main-task stack.
// Reserve substantially more than that implementation detail so painting never
// touches the guard/control area.
const STACK_WATERMARK_RESERVED_BYTES: usize = 256;
// Never paint right up to the live SP; leave room for this function and its
// caller to return without touching freshly painted memory.
const STACK_WATERMARK_SAFETY_BYTES: usize = 256;
const STACK_WORD_BYTES: usize = core::mem::size_of::<u32>();

static CPU0_WATERMARK_START: AtomicUsize = AtomicUsize::new(0);
static CPU0_WATERMARK_END: AtomicUsize = AtomicUsize::new(0);
static CPU0_MIN_HEADROOM: AtomicUsize = AtomicUsize::new(0);

static CPU1_STACK_BOTTOM: AtomicUsize = AtomicUsize::new(0);
static CPU1_STACK_TOP: AtomicUsize = AtomicUsize::new(0);
static CPU1_WATERMARK_START: AtomicUsize = AtomicUsize::new(0);
static CPU1_WATERMARK_END: AtomicUsize = AtomicUsize::new(0);
static CPU1_MIN_HEADROOM: AtomicUsize = AtomicUsize::new(0);

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
pub(crate) fn init_cpu0_stack_watermark() {
    let (bottom, top) = cpu0_stack_bounds();
    paint_live_stack(
        bottom,
        top,
        &CPU0_WATERMARK_START,
        &CPU0_WATERMARK_END,
        &CPU0_MIN_HEADROOM,
    );
}

/// Record the statically allocated CPU1 stack before it is handed to ESP-RTOS.
pub(crate) fn register_cpu1_stack<const SIZE: usize>(stack: &mut Stack<SIZE>) {
    CPU1_STACK_BOTTOM.store(stack.bottom() as usize, Ordering::Release);
    CPU1_STACK_TOP.store(stack.top() as usize, Ordering::Release);
}

/// Paint CPU1's currently-unused stack. Call this from the CPU1 entry closure.
pub(crate) fn init_cpu1_stack_watermark() {
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

pub(super) fn update_cpu0_stack_watermark() {
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

/// CPU1 scans its own watermark so CPU0 never reads a stack CPU1 may be using.
#[embassy_executor::task]
pub(crate) async fn cpu1_stack_monitor_task() {
    loop {
        update_cpu1_stack_watermark();
        Timer::after(PERIODIC_REPORT_INTERVAL).await;
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Snapshot {
    pub(super) size: usize,
    pub(super) headroom: usize,
}

pub(super) fn cpu0_snapshot() -> Snapshot {
    let sp = esp_hal::xtensa_lx::get_stack_pointer() as usize;
    let (bottom, top) = cpu0_stack_bounds();
    Snapshot {
        size: top.saturating_sub(bottom),
        headroom: sp.saturating_sub(bottom),
    }
}

pub(super) fn cpu0_min_headroom() -> usize {
    CPU0_MIN_HEADROOM.load(Ordering::Acquire)
}

pub(super) fn cpu1_size() -> usize {
    CPU1_STACK_TOP
        .load(Ordering::Acquire)
        .saturating_sub(CPU1_STACK_BOTTOM.load(Ordering::Acquire))
}

pub(super) fn cpu1_min_headroom() -> usize {
    CPU1_MIN_HEADROOM.load(Ordering::Acquire)
}
