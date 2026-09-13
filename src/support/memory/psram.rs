//! Dedicated external-PSRAM allocator policy.

use esp_alloc::{EspHeap, HeapRegion, MemoryCapability};
use esp_hal::{
    peripherals::PSRAM,
    psram::{Psram, PsramConfig, PsramMode},
};

static PSRAM_HEAP: EspHeap = EspHeap::empty();

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

pub(super) fn heap() -> &'static EspHeap {
    &PSRAM_HEAP
}
