//! Long-lived buffers in PSRAM.
//!
//! The ESP32-S3 has two kinds of RAM:
//!
//! - **Internal RAM**: fast, but small. The firmware gets two heaps of about
//!   72 KiB each, and every task stack lives here.
//! - **PSRAM**: an external 8 MiB chip, slower to access but plentiful.
//!
//! PSRAM gets its own heap instead of joining the global allocator, so a
//! large buffer only lands there when the code asks for it through the two
//! helpers below, and ordinary allocations stay in fast internal RAM. Do not
//! put large arrays on the stack: a 320x240 frame is 150 KiB.
//!
//! Both helpers allocate once and never free: they are for buffers that live
//! as long as the device runs, such as a canvas or a camera frame. Because
//! nothing is ever freed, the returned references are `'static` and can be
//! stored in any struct.
//!
//! Anything that talks to hardware directly (DMA descriptors, task stacks,
//! synchronization objects) stays in internal RAM.
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

/// Map the PSRAM chip into the address space and hand it to the PSRAM heap.
pub(crate) fn enable(psram_peripheral: PSRAM<'static>) {
    let config = PsramConfig {
        mode: PsramMode::QuadSpi,
        ..PsramConfig::default()
    };

    let psram = Psram::new(psram_peripheral, config);
    let (start, size) = psram.raw_parts();

    // SAFETY: `psram` is the unique owner created from the singleton PSRAM
    // peripheral. `raw_parts()` describes that initialized external-memory
    // region exactly once, and this module keeps the only `EspHeap` that will
    // ever register or allocate from it. The device-lifetime heap outlives all
    // allocations made through `leaked_slice` and `leaked_value`.
    unsafe {
        PSRAM_HEAP.add_region(HeapRegion::new(
            start,
            size,
            MemoryCapability::External.into(),
        ));
    }
}

/// The PSRAM heap, for allocations that should live there.
pub(crate) fn heap() -> &'static EspHeap {
    &PSRAM_HEAP
}

/// A slice of `len` copies of `value` in PSRAM.
///
/// # Panics
///
/// When PSRAM is exhausted.
pub fn leaked_slice<T: Clone + 'static>(len: usize, value: T) -> &'static mut [T] {
    let mut storage = Vec::with_capacity_in(len, heap());
    storage.resize(len, value);
    storage.leak()
}

/// One value, built by `init` and moved into PSRAM.
///
/// Use it for objects that should not live in a struct on a task's stack
/// because they are large. `init` still builds the value on the calling
/// stack before it moves, so the caller's stack needs room for it once.
///
/// # Panics
///
/// When PSRAM is exhausted.
pub fn leaked_value<T: 'static>(init: impl FnOnce() -> T) -> &'static mut T {
    let mut storage = Box::<T, _>::new_uninit_in(heap());
    // SAFETY: `storage` owns one properly aligned, uninitialized allocation for
    // exactly one `T`. The value is written exactly once and no initialized
    // reference exists before that write.
    unsafe {
        storage.as_mut_ptr().write(init());
        Box::leak(storage.assume_init())
    }
}
