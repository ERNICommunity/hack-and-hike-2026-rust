//! Long-lived buffers in PSRAM.
//!
//! The ESP32-S3 has two kinds of RAM:
//!
//! - **Internal RAM**: fast, but small. The firmware has two heaps of about
//!   72 KiB each. Every task stack is in internal RAM too.
//! - **PSRAM** (pseudo-static RAM): an external 8 MiB chip. It is slower, but
//!   it has much more space.
//!
//! PSRAM has its own heap. It is not part of the global allocator. So a
//! buffer is in PSRAM only when the code asks for it with one of the two
//! functions below. Normal allocations, such as `Box` and `Vec`, stay in fast
//! internal RAM. Do not put large arrays on the stack: one 320x240 frame is
//! 150 KiB.
//!
//! Both functions allocate once and never free the memory. Use them for
//! buffers that exist as long as the device runs, such as a canvas or a
//! camera frame. Because the memory is never freed, the returned references
//! are `'static`, and you can store them in any struct.
//!
//! Some memory stays in internal RAM: DMA descriptors, which the hardware
//! uses directly, task stacks and synchronization objects. DMA (direct memory
//! access) lets a peripheral read or write memory without the CPU.
//!
//! ```ignore
//! let samples: &'static mut [i16] = psram::leaked_slice(16_000, 0);
//! ```

use allocator_api2::{boxed::Box, vec::Vec};
use esp_alloc::{EspHeap, HeapRegion, MemoryCapability};
use esp_hal::{
    peripherals::PSRAM,
    psram::{Psram, PsramConfig, PsramMode},
};

/// The allocator over PSRAM; empty until [`enable`] adds the memory.
static PSRAM_HEAP: EspHeap = EspHeap::empty();

/// Map the PSRAM chip into the address space and give its memory to the
/// PSRAM heap. [`Board::init`](crate::Board::init) calls it once.
pub(crate) fn enable(psram_peripheral: PSRAM<'static>) {
    let config = PsramConfig {
        mode: PsramMode::QuadSpi,
        ..PsramConfig::default()
    };

    let psram = Psram::new(psram_peripheral, config);
    let (start, size) = psram.raw_parts();

    // SAFETY: `psram` is created from the PSRAM peripheral, which exists only
    // once, so no other code owns this memory. `raw_parts()` returns the start
    // and size of the mapped external memory. The region is added exactly
    // once, and only to `PSRAM_HEAP`, the one heap in this module. No other
    // heap uses or allocates from it. `PSRAM_HEAP` is a `static`, so it
    // exists longer than every allocation from `leaked_slice` and
    // `leaked_value`.
    unsafe {
        PSRAM_HEAP.add_region(HeapRegion::new(
            start,
            size,
            MemoryCapability::External.into(),
        ));
    }
}

/// The PSRAM heap. [`leaked_slice`] and [`leaked_value`] allocate from it,
/// and [`report_memory`](crate::logging::report_memory) reads its usage.
pub(crate) fn heap() -> &'static EspHeap {
    &PSRAM_HEAP
}

/// Allocate a slice of `len` copies of `value` in PSRAM. The memory is never
/// freed.
///
/// # Panics
///
/// When PSRAM does not have enough free memory, or before
/// [`Board::init`](crate::Board::init) has run.
pub fn leaked_slice<T: Clone + 'static>(len: usize, value: T) -> &'static mut [T] {
    let mut storage = Vec::with_capacity_in(len, heap());
    storage.resize(len, value);
    storage.leak()
}

/// Build one value with `init` and move it into PSRAM. The memory is never
/// freed.
///
/// Use it for a value that is too large for a task's stack. Note that `init`
/// may build the value on the calling stack before it moves to PSRAM. So the
/// calling stack still needs room for the value once.
///
/// # Panics
///
/// When PSRAM does not have enough free memory, or before
/// [`Board::init`](crate::Board::init) has run.
pub fn leaked_value<T: 'static>(init: impl FnOnce() -> T) -> &'static mut T {
    let mut storage = Box::<T, _>::new_uninit_in(heap());
    // SAFETY: `storage` owns one correctly aligned, uninitialized allocation
    // for exactly one `T`. `write` stores the value without reading or
    // dropping the old, uninitialized content. No reference to the memory
    // exists before the write. After the write the value is fully
    // initialized, so `assume_init` is correct.
    unsafe {
        storage.as_mut_ptr().write(init());
        Box::leak(storage.assume_init())
    }
}
