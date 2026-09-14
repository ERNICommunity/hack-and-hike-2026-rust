//! The PSRAM heap.
//!
//! PSRAM gets its own heap instead of joining the global allocator, so a
//! large buffer can only land there when the code asks for it explicitly
//! (through [`super::storage`]), and ordinary allocations stay in fast
//! internal RAM.

use esp_alloc::{EspHeap, HeapRegion, MemoryCapability};
use esp_hal::{
    peripherals::PSRAM,
    psram::{Psram, PsramConfig, PsramMode},
};

/// The allocator over PSRAM; empty until [`enable`] adds the memory.
static PSRAM_HEAP: EspHeap = EspHeap::empty();

/// Map the PSRAM chip into the address space and hand it to the PSRAM heap.
pub fn enable(psram_peripheral: PSRAM<'static>) {
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
    // allocations made through `storage`.
    unsafe {
        PSRAM_HEAP.add_region(HeapRegion::new(
            start,
            size,
            MemoryCapability::External.into(),
        ));
    }
}

/// The PSRAM heap, for allocations that should live there.
pub(super) fn heap() -> &'static EspHeap {
    &PSRAM_HEAP
}
