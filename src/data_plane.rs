//! Explicit PSRAM-backed bulk storage.
//!
//! Every collection in this module allocates its backing storage once during
//! construction and then operates within a fixed bound. These types are for
//! plain bulk data such as log bytes and RGB565 framebuffer pixels. Runtime
//! handles, synchronization objects, atomics, and DMA state remain in internal
//! RAM.

use allocator_api2::vec::Vec;
use esp_alloc::EspHeap;

use crate::memory;

pub type PsramVec<T> = Vec<T, &'static EspHeap>;

pub fn vec_with_capacity<T>(capacity: usize) -> PsramVec<T> {
    Vec::with_capacity_in(capacity, memory::psram_heap())
}

pub fn zeroed_bytes(len: usize) -> PsramVec<u8> {
    let mut bytes = vec_with_capacity(len);
    bytes.resize(len, 0);
    bytes
}

/// Allocate fixed PSRAM storage whose lifetime intentionally matches the device.
///
/// This is for APIs such as framebuffer/DMA abstractions that require a static
/// backing slice. The allocation happens once during bootstrap and is never
/// replaced or resized afterwards.
pub fn leaked_filled_slice<T: Clone + 'static>(len: usize, value: T) -> &'static mut [T] {
    let mut storage = vec_with_capacity(len);
    storage.resize(len, value);
    storage.leak()
}

/// Fixed-size PSRAM-backed storage. Capacity is established once and never
/// changes afterwards.
pub struct FixedPsramBuffer<T> {
    storage: PsramVec<T>,
}

impl<T: Clone> FixedPsramBuffer<T> {
    pub fn filled(len: usize, value: T) -> Self {
        let mut storage = vec_with_capacity(len);
        storage.resize(len, value);
        Self { storage }
    }
}

impl<T> FixedPsramBuffer<T> {
    pub fn as_slice(&self) -> &[T] {
        &self.storage
    }

    pub fn as_mut_slice(&mut self) -> &mut [T] {
        &mut self.storage
    }
}

/// Fixed-capacity byte ring whose backing bytes live in PSRAM.
pub struct PsramByteRing {
    storage: PsramVec<u8>,
    start: usize,
    len: usize,
}

impl PsramByteRing {
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0);
        Self {
            storage: zeroed_bytes(capacity),
            start: 0,
            len: 0,
        }
    }

    pub fn capacity(&self) -> usize {
        self.storage.len()
    }

    pub fn len(&self) -> usize {
        self.len
    }

    fn drop_oldest_line(&mut self) {
        if self.len == 0 {
            return;
        }

        let capacity = self.capacity();
        let mut dropped = 0usize;

        while dropped < self.len {
            let index = (self.start + dropped) % capacity;
            dropped += 1;
            if self.storage[index] == b'\n' {
                break;
            }
        }

        self.start = (self.start + dropped) % capacity;
        self.len -= dropped;

        if self.len == 0 {
            self.start = 0;
        }
    }

    pub fn push_line(&mut self, bytes: &[u8]) -> bool {
        if bytes.is_empty() || bytes.len() > self.capacity() {
            return false;
        }

        while self.len + bytes.len() > self.capacity() {
            self.drop_oldest_line();
        }

        let capacity = self.capacity();
        let end = (self.start + self.len) % capacity;
        let first_len = bytes.len().min(capacity - end);
        self.storage[end..end + first_len].copy_from_slice(&bytes[..first_len]);

        let remaining = bytes.len() - first_len;
        if remaining != 0 {
            self.storage[..remaining].copy_from_slice(&bytes[first_len..]);
        }

        self.len += bytes.len();
        true
    }

    /// Copy a logical range from the oldest byte onward without exposing the
    /// ring's physical wrap point.
    pub fn copy_range_to(&self, logical_offset: usize, out: &mut [u8]) -> usize {
        if logical_offset >= self.len || out.is_empty() {
            return 0;
        }

        let count = (self.len - logical_offset).min(out.len());
        let capacity = self.capacity();
        let physical_start = (self.start + logical_offset) % capacity;
        let first_len = count.min(capacity - physical_start);

        out[..first_len]
            .copy_from_slice(&self.storage[physical_start..physical_start + first_len]);

        let remaining = count - first_len;
        if remaining != 0 {
            out[first_len..count].copy_from_slice(&self.storage[..remaining]);
        }

        count
    }
}
