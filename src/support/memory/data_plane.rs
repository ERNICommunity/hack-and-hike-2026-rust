//! Explicit PSRAM-backed bulk storage.
//!
//! Every collection in this module allocates its backing storage once during
//! construction and then operates within a fixed bound. These types are for
//! presentation/data-plane state that does not require internal RAM. Runtime
//! handles, synchronization objects, stacks, atomics, and DMA descriptors remain
//! in internal RAM.

use allocator_api2::{boxed::Box, vec::Vec};
use esp_alloc::EspHeap;

use super::psram;

pub(crate) type PsramVec<T> = Vec<T, &'static EspHeap>;

pub(crate) fn vec_with_capacity<T>(capacity: usize) -> PsramVec<T> {
    Vec::with_capacity_in(capacity, psram::heap())
}

pub(crate) fn zeroed_bytes(len: usize) -> PsramVec<u8> {
    let mut bytes = vec_with_capacity(len);
    bytes.resize(len, 0);
    bytes
}

/// Allocate fixed PSRAM storage whose lifetime intentionally matches the device.
///
/// This is for APIs such as framebuffer/DMA abstractions that require a static
/// backing slice. The allocation happens once during bootstrap and is never
/// replaced or resized afterwards.
pub(crate) fn leaked_filled_slice<T: Clone + 'static>(len: usize, value: T) -> &'static mut [T] {
    let mut storage = vec_with_capacity(len);
    storage.resize(len, value);
    storage.leak()
}

/// Construct one long-lived object directly in explicitly allocated PSRAM.
///
/// Large fixed-capacity presentation structures should not become members of an
/// async task's stack frame merely because they need a `'static` lifetime. The
/// backing allocation is intentionally leaked because device-lifetime UI/data
/// state has no meaningful teardown path.
///
/// `new_uninit_in` establishes the final aligned PSRAM destination first. The
/// initializer is then written into that destination exactly once before the
/// allocation is exposed as initialized `T`.
pub(crate) fn leaked_value_with<T: 'static>(init: impl FnOnce() -> T) -> &'static mut T {
    let mut storage = Box::<T, _>::new_uninit_in(psram::heap());
    // SAFETY: `storage` owns one properly aligned, uninitialized allocation for
    // exactly one `T`. `init()` is evaluated before the write, its value is
    // written exactly once, and no initialized reference is created until after
    // that write. The box is then intentionally leaked for device lifetime.
    unsafe {
        storage.as_mut_ptr().write(init());
        Box::leak(storage.assume_init())
    }
}

/// Fixed-size PSRAM-backed storage. Capacity is established once and never
/// changes afterwards.
pub(crate) struct FixedPsramBuffer<T> {
    storage: PsramVec<T>,
}

impl<T: Clone> FixedPsramBuffer<T> {
    pub(crate) fn filled(len: usize, value: T) -> Self {
        let mut storage = vec_with_capacity(len);
        storage.resize(len, value);
        Self { storage }
    }
}

impl<T> FixedPsramBuffer<T> {
    pub(crate) fn as_slice(&self) -> &[T] {
        &self.storage
    }

    pub(crate) fn as_mut_slice(&mut self) -> &mut [T] {
        &mut self.storage
    }
}

/// Fixed-capacity byte ring whose backing bytes live in PSRAM.
pub(crate) struct PsramByteRing {
    storage: PsramVec<u8>,
    start: usize,
    len: usize,
}

impl PsramByteRing {
    pub(crate) fn new(capacity: usize) -> Self {
        assert!(capacity > 0);
        Self {
            storage: zeroed_bytes(capacity),
            start: 0,
            len: 0,
        }
    }

    pub(crate) fn capacity(&self) -> usize {
        self.storage.len()
    }

    pub(crate) fn len(&self) -> usize {
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

    pub(crate) fn push_line(&mut self, bytes: &[u8]) -> bool {
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

    pub(crate) fn copy_range_to(&self, logical_offset: usize, out: &mut [u8]) -> usize {
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
