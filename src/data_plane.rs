//! Explicit PSRAM-backed application data structures.
//!
//! Every collection in this module allocates its backing storage once during
//! construction and then operates within a compile-time/runtime bound. These
//! types are appropriate for plain data only; UI/runtime/atomic/DMA objects
//! stay in internal RAM.

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

/// Fixed-capacity ring whose complete backing array is allocated once in
/// PSRAM. Pushing when full overwrites the oldest element; it never reallocates.
pub struct FixedPsramRing<T: Copy + Default, const N: usize> {
    storage: PsramVec<T>,
    start: usize,
    len: usize,
}

impl<T: Copy + Default, const N: usize> FixedPsramRing<T, N> {
    pub fn new() -> Self {
        assert!(N > 0);

        let mut storage = vec_with_capacity(N);
        storage.resize(N, T::default());

        Self {
            storage,
            start: 0,
            len: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn capacity(&self) -> usize {
        N
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn clear(&mut self) {
        self.start = 0;
        self.len = 0;
    }

    pub fn push_back(&mut self, value: T) {
        if self.len < N {
            let index = (self.start + self.len) % N;
            self.storage[index] = value;
            self.len += 1;
        } else {
            self.storage[self.start] = value;
            self.start = (self.start + 1) % N;
        }
    }

    pub fn get(&self, logical_index: usize) -> Option<&T> {
        if logical_index >= self.len {
            return None;
        }

        let index = (self.start + logical_index) % N;
        self.storage.get(index)
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

    pub fn clear(&mut self) {
        self.start = 0;
        self.len = 0;
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

    pub fn copy_to(&self, out: &mut [u8]) -> usize {
        let count = self.len.min(out.len());
        if count == 0 {
            return 0;
        }

        let capacity = self.capacity();
        let first_len = count.min(capacity - self.start);
        out[..first_len].copy_from_slice(&self.storage[self.start..self.start + first_len]);

        let remaining = count - first_len;
        if remaining != 0 {
            out[first_len..count].copy_from_slice(&self.storage[..remaining]);
        }

        count
    }
}
