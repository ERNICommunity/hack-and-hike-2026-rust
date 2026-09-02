//! Application memory layout.
//!
//! Ordinary Rust/Slint/radio allocations continue to use `esp_alloc::HEAP`,
//! which contains only internal RAM regions registered by `main.rs`.
//!
//! The CoreS3-Lite PSRAM is mapped into a *separate* allocator. This is
//! intentional: on ESP32-S3 atomic instructions do not work correctly when
//! their target lives in PSRAM, so opaque runtime objects must never spill
//! there merely because the internal heap is full.
//!
//! Future large, plain-data allocations (network payload storage, peer-history
//! buffers, images, file caches, etc.) can opt into this allocator explicitly.

use esp_alloc::{EspHeap, HEAP, HeapRegion, MemoryCapability};
use esp_hal::{
    peripherals::PSRAM,
    psram::{Psram, PsramConfig, PsramMode},
};

/// Dedicated external-memory allocator.
///
/// This is deliberately NOT the application's `#[global_allocator]`.
static PSRAM_HEAP: EspHeap = EspHeap::empty();

/// Initialize/map the CoreS3-Lite PSRAM and register it only with the dedicated
/// external-memory heap.
///
/// The CoreS3-Lite uses Quad-SPI PSRAM, so configure that explicitly instead of
/// `Auto`. This avoids the ESP32-S3 Octal-mode probe, whose GPIOs overlap the
/// CoreS3 LCD pins.
///
/// Call this before configuring the CoreS3 LCD so display GPIO/SPI setup is the
/// final peripheral configuration affecting those pins.
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

    ::log::info!(
        "Memory ready: internal heap={} KiB, dedicated PSRAM={} KiB",
        internal_free_bytes() / 1024,
        psram_free_bytes() / 1024,
    );
}

/// The allocator to use for explicit PSRAM-backed allocations.
///
/// Do not use it for types containing atomics, locks, RTOS/runtime state,
/// DMA descriptors/buffers, or other synchronization primitives.
pub fn psram_heap() -> &'static EspHeap {
    &PSRAM_HEAP
}

/// Free bytes currently available from the normal internal global heap.
pub fn internal_free_bytes() -> usize {
    HEAP.free_caps(MemoryCapability::Internal.into())
}

/// Free bytes currently available from the dedicated PSRAM heap.
pub fn psram_free_bytes() -> usize {
    PSRAM_HEAP.free()
}
